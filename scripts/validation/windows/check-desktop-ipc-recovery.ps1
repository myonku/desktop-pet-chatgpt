#Requires -Version 7.4
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateSet('UnconfirmedSend','InFlightReadReconnect')][string] $Mode,
    [Parameter(Mandatory)][ValidatePattern('^[A-Za-z0-9_-]{1,128}\z')][string] $CodexTestId,
    [Parameter(Mandatory)][string] $OutputDirectory,
    [ValidateRange(10,120)][int] $TimeoutSeconds = 120,
    [ValidatePattern('^DESKTOP_PET_M6_CLIENT_RECOVERY_[A-Z0-9_]{2,32}\z')][string] $ClientTestMarker = 'DESKTOP_PET_M6_CLIENT_RECOVERY_01'
)
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot desktop_ipc.psm1) -Force
# Existing receipts must not be replaced: an interrupted process may have sent.
if (Test-Path -LiteralPath (Join-Path $OutputDirectory result.json)) { throw 'recovery_receipt_already_exists' }
$outputPath = (New-Item -ItemType Directory -Path $OutputDirectory -Force).FullName
$report = [ordered]@{ schema_version = 1; status = 'error'; probe_scope = $Mode; desktop_ui_verified = $false
    write_attempts = 0; automatic_resends = 0; disconnected_own_probe_only = $true; reconnected = $false }
$session = $null
$stage = 'initial_snapshot'
function Save-Receipt { $report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $outputPath result.json) }
function Marked-Turns($State) {
    return @(Get-DesktopProbeTurns $State | Where-Object {
        @($_.params.input | Where-Object { $_.type -ceq 'text' -and $_.text.Contains($ClientTestMarker) }).Count -gt 0
    })
}
try {
    $session = Open-DesktopProbeSession $CodexTestId
    $initialPeer = $session.peerGeneration
    $initialOwner = $session.owner
    $report.desktop_version = $session.desktopVersion
    $report.desktop_peer_verified = $true
    $snapshot = Get-DesktopProbeSessionSnapshot $session
    $current = @{ revision = $snapshot.revision; conversationState = $snapshot.conversationState }
    if ($Mode -ceq 'UnconfirmedSend') {
        Assert-DesktopProbeTestWriteReady $current.conversationState
        $marker = 'DESKTOP_PET_M6_IPC_' + [guid]::NewGuid().ToString('N')
        $messageId = [guid]::NewGuid().ToString()
        $report.correlation = @{ expected_test_marker = $marker; client_message_id = $messageId }
        $report.write_attempts = 1
        $report.acknowledgement_read = $false
        $report.delivery = 'unconfirmed'
        $stage = 'send_without_reading_acknowledgement'
        Save-Receipt
        Send-DesktopProbeTestFrame $session.pipe $CodexTestId $marker $messageId $session.client $session.owner -AllowTestTurn
    } else {
        if (@(Marked-Turns $current.conversationState).Count) { throw 'client_test_already_present' }
        $stage = 'wait_for_partial_client_reply'
        Write-Host 'READY: waiting for a new marked client reply; the probe will disconnect after partial text.'
        $deadline = [Threading.CancellationTokenSource]::new($TimeoutSeconds * 1000)
        try {
            while ($true) {
                $change = Receive-DesktopProbeSessionChange $session $deadline.Token
                $null = Update-DesktopProbeStream $current $change $CodexTestId
                $turns = @(Marked-Turns $current.conversationState)
                if ($turns.Count -ne 1) { continue }
                $turn = $turns[0]
                if ($turn.status -ceq 'inProgress' -and $turn.turnId -and
                    @($turn.items | Where-Object { $_.type -ceq 'agentMessage' -and $_.text }).Count) {
                    if (@($turn.items | Where-Object { $_.type -cnotin @('userMessage','agentMessage','reasoning') }).Count) { throw 'test_has_tools' }
                    $targetTurn = $turn.turnId
                    $partialText = @($turn.items | Where-Object { $_.type -ceq 'agentMessage' } | ForEach-Object { $_.text }) -join ''
                    $report.target_turn_id = $targetTurn
                    $report.partial_text_before_disconnect = $true
                    $report.active_before_disconnect = $true
                    break
                }
            }
        } finally { $deadline.Dispose() }
    }
    $stage = 'abrupt_disconnect'
    Close-DesktopProbeSession $session -Abrupt
    $session = $null
    Save-Receipt
    $stage = 'reconnect_and_revalidate'
    $session = Open-DesktopProbeSession $CodexTestId
    $report.reconnected = $true
    $report.desktop_peer_reverified = $true
    $report.same_desktop_process = $session.peerGeneration -ceq $initialPeer
    $report.same_owner = $session.owner -ceq $initialOwner
    $snapshot = Get-DesktopProbeSessionSnapshot $session
    $current = @{ revision = $snapshot.revision; conversationState = $snapshot.conversationState }
    $report.fresh_snapshot_after_reconnect = $true
    $stage = 'recover_original_turn_without_resend'
    $deadline = [Threading.CancellationTokenSource]::new($TimeoutSeconds * 1000)
    try {
        $report.post_reconnect_changes = 0
        while ($true) {
            if ($Mode -ceq 'UnconfirmedSend') {
                $matches = @(Get-DesktopProbeTurns $current.conversationState | Where-Object { $_.params.clientUserMessageId -ceq $messageId })
                if ($matches.Count -gt 1) { throw 'duplicate_test_message' }
                if ($matches.Count -eq 1) {
                    $report.delivery = 'observed_in_original_thread'
                    if ($matches[0].status -ceq 'completed') {
                        $report.verification = Test-DesktopProbeReply $current.conversationState $CodexTestId $marker $messageId
                        if ($false -cin $report.verification.Values) { throw 'test_reply_unverified' }
                        $report.exactly_one_matching_turn = $true
                        break
                    }
                    if ($matches[0].status -cin @('failed','interrupted')) { throw 'test_turn_not_completed' }
                }
            } else {
                $matches = @(Marked-Turns $current.conversationState)
                if ($matches.Count -ne 1 -or $matches[0].turnId -cne $targetTurn) { throw 'recovered_turn_mismatch' }
                if (-not $report.Contains('active_after_reconnect')) { $report.active_after_reconnect = $matches[0].status -ceq 'inProgress' }
                if ($matches[0].status -ceq 'completed') {
                    $report.same_turn_completed = $true
                    $report.no_tool_items = @($matches[0].items | Where-Object { $_.type -cnotin @('userMessage','agentMessage','reasoning') }).Count -eq 0
                    $report.agent_reply_present = @($matches[0].items | Where-Object { $_.type -ceq 'agentMessage' -and $_.text }).Count -gt 0
                    $completedText = @($matches[0].items | Where-Object { $_.type -ceq 'agentMessage' } | ForEach-Object { $_.text }) -join ''
                    $report.partial_prefix_retained = $completedText.StartsWith($partialText,[StringComparison]::Ordinal)
                    if (-not $report.no_tool_items -or -not $report.agent_reply_present -or -not $report.partial_prefix_retained) { throw 'test_reply_unverified' }
                    break
                }
                if ($matches[0].status -cin @('failed','interrupted')) { throw 'test_turn_not_completed' }
            }
            Save-Receipt
            $change = Receive-DesktopProbeSessionChange $session $deadline.Token
            $null = Update-DesktopProbeStream $current $change $CodexTestId
            $report.post_reconnect_changes++
        }
    } finally { $deadline.Dispose() }
    $final = Get-DesktopProbeSessionSnapshot $session
    if ($Mode -ceq 'UnconfirmedSend') {
        $verified = Test-DesktopProbeReply $final.conversationState $CodexTestId $marker $messageId
        if ($false -cin $verified.Values) { throw 'test_reply_unverified' }
    } else {
        $matches = @(Marked-Turns $final.conversationState)
        if ($matches.Count -ne 1 -or $matches[0].turnId -cne $targetTurn -or $matches[0].status -cne 'completed') { throw 'recovered_turn_mismatch' }
        $finalText = @($matches[0].items | Where-Object { $_.type -ceq 'agentMessage' } | ForEach-Object { $_.text }) -join ''
        $report.final_text_matches_stream = $finalText -ceq $completedText
        if (-not $report.final_text_matches_stream) { throw 'test_reply_unverified' }
    }
    $report.final_snapshot_verified = $true
    $report.status = 'ok'
} catch {
    $report.failed_stage = $stage
    $report.failure_type = $_.Exception.GetType().Name
    $report.failure_id = $_.FullyQualifiedErrorId
    $report.failure_line = $_.InvocationInfo.ScriptLineNumber
    $reason = $_.Exception.GetBaseException().Message
    if ($reason -cin @('unsupported_desktop_package','codex_owner_required','unexpected_pipe_peer','snapshot_identity_mismatch',
        'test_thread_not_ready','test_write_already_attempted','client_test_already_present','test_has_tools','owner_disconnected',
        'unsupported_stream_version','stream_revision_gap','duplicate_test_message','test_reply_unverified',
        'test_turn_not_completed','recovered_turn_mismatch')) { $report.failure_reason = $reason }
} finally {
    Close-DesktopProbeSession $session
    Save-Receipt
    $report | ConvertTo-Json -Depth 8
}
if ($report.status -cne 'ok') { exit 1 }
