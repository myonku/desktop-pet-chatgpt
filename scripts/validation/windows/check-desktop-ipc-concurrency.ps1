#Requires -Version 7.4
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidatePattern('^[A-Za-z0-9_-]{1,128}\z')][string] $CodexTestId,
    [Parameter(Mandatory)][string] $OutputDirectory,
    [ValidateRange(10,120)][int] $TimeoutSeconds = 120
)
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot desktop_ipc.psm1) -Force
if (Test-Path -LiteralPath (Join-Path $OutputDirectory result.json)) { throw 'concurrency_receipt_already_exists' }
$outputPath = (New-Item -ItemType Directory -Path $OutputDirectory -Force).FullName
$report = [ordered]@{ schema_version = 1; status = 'error'; probe_scope = 'two_follower_writes'
    desktop_ui_verified = $false; write_attempts = 0; automatic_resends = 0; requests_sent_before_any_ack_read = $false }
$sessions = @()
$receipts = @()
$stage = 'prepare_two_writers'
function Save-Receipt { $report | ConvertTo-Json -Depth 9 | Set-Content -LiteralPath (Join-Path $outputPath result.json) }
try {
    foreach ($index in 0..1) {
        if ($index -eq 0) { $sessions += Open-DesktopProbeSession $CodexTestId }
        else { $sessions += Open-DesktopProbeSession $CodexTestId -OwnerHint $sessions[0].owner }
        $snapshot = Get-DesktopProbeSessionSnapshot $sessions[$index]
        Assert-DesktopProbeTestWriteReady $snapshot.conversationState
        $receipts += [ordered]@{ expected_test_marker = ('DESKTOP_PET_M6_IPC_' + [guid]::NewGuid().ToString('N'))
            client_message_id = [guid]::NewGuid().ToString(); request_id = [guid]::NewGuid().ToString()
            send_attempted = $false; acknowledgement = 'unconfirmed' }
    }
    $report.desktop_version = $sessions[0].desktopVersion
    $report.desktop_peer_verified = $true
    $report.independent_clients = $sessions[0].client -cne $sessions[1].client
    $report.same_owner = $sessions[0].owner -ceq $sessions[1].owner
    $report.same_desktop_process = $sessions[0].peerGeneration -ceq $sessions[1].peerGeneration
    if (-not $report.independent_clients -or -not $report.same_owner -or -not $report.same_desktop_process) { throw 'writer_binding_mismatch' }
    $report.requests = $receipts
    $stage = 'write_two_requests'
    $timer = [Diagnostics.Stopwatch]::StartNew()
    foreach ($index in 0..1) {
        $report.write_attempts++
        $receipts[$index].send_attempted = $true
        Save-Receipt
        Send-DesktopProbeTestFrame $sessions[$index].pipe $CodexTestId $receipts[$index].expected_test_marker `
            $receipts[$index].client_message_id $sessions[$index].client $sessions[$index].owner `
            -RequestId $receipts[$index].request_id -AllowTestTurn
        $receipts[$index].frame_written_at_ms = $timer.ElapsedMilliseconds
    }
    $report.frame_write_interval_ms = $receipts[1].frame_written_at_ms - $receipts[0].frame_written_at_ms
    $report.requests_sent_before_any_ack_read = $true
    Save-Receipt
    $stage = 'read_acknowledgements'
    foreach ($index in 0..1) {
        $reply = Receive-DesktopProbeTestAcknowledgement $sessions[$index] $receipts[$index].request_id
        $receipts[$index].acknowledgement = $reply.resultType
        if ($reply.resultType -ceq 'success') {
            $turn = $reply.result.result.turn.id
            if ($turn -isnot [string] -or -not $turn) { throw 'test_turn_ack_unrecognized' }
            $receipts[$index].ack_turn_id = $turn
        } else {
            $receipts[$index].rejected = $true
            $receipts[$index].error_value_type = if ($null -eq $reply.error) { 'null' } else { $reply.error.GetType().Name }
        }
        Save-Receipt
    }
    Close-DesktopProbeSession $sessions[1]
    $sessions = @($sessions[0])
    $stage = 'verify_correlated_history'
    $snapshot = Get-DesktopProbeSessionSnapshot $sessions[0]
    $current = @{ revision = $snapshot.revision; conversationState = $snapshot.conversationState }
    $deadline = [Threading.CancellationTokenSource]::new($TimeoutSeconds * 1000)
    try {
        while ($true) {
            $allTerminal = $true
            foreach ($receipt in $receipts) {
                $matches = @(Get-DesktopProbeTurns $current.conversationState | Where-Object { $_.params.clientUserMessageId -ceq $receipt.client_message_id })
                $receipt.matching_turns = $matches.Count
                if ($matches.Count -gt 1) { throw 'duplicate_test_message' }
                if ($receipt.acknowledgement -ceq 'success' -and $matches.Count -ne 1) {
                    $acknowledged = @(Get-DesktopProbeTurns $current.conversationState | Where-Object { $_.turnId -ceq $receipt.ack_turn_id })
                    if ($current.conversationState.threadRuntimeStatus.type -ceq 'idle' -and $acknowledged.Count -eq 1 -and
                        $acknowledged[0].status -cin @('completed','failed','interrupted')) { $receipt.missing_message_correlation = $true }
                    else { $allTerminal = $false }
                    continue
                }
                if ($matches.Count -eq 1) {
                    $receipt.turn_status = $matches[0].status
                    $receipt.ack_turn_matches = $matches[0].turnId -ceq $receipt.ack_turn_id
                    if (-not $matches[0].turnId -and $matches[0].status -ceq 'inProgress' -and
                        $current.conversationState.threadRuntimeStatus.type -ceq 'idle') {
                        $acknowledged = @(Get-DesktopProbeTurns $current.conversationState | Where-Object { $_.turnId -ceq $receipt.ack_turn_id })
                        if ($acknowledged.Count -eq 1 -and $acknowledged[0].status -cin @('completed','failed','interrupted')) {
                            $receipt.dangling_optimistic_turn = $true
                            continue
                        }
                    }
                    if ($matches[0].status -cnotin @('completed','failed','interrupted')) { $allTerminal = $false; continue }
                    $receipt.verification = Test-DesktopProbeReply $current.conversationState $CodexTestId $receipt.expected_test_marker $receipt.client_message_id
                }
            }
            if ($allTerminal -and $current.conversationState.threadRuntimeStatus.type -ceq 'idle') { break }
            Save-Receipt
            $change = Receive-DesktopProbeSessionChange $sessions[0] $deadline.Token
            $null = Update-DesktopProbeStream $current $change $CodexTestId
        }
    } finally { $deadline.Dispose() }
    $final = Get-DesktopProbeSessionSnapshot $sessions[0]
    $valid = 0
    foreach ($receipt in $receipts) {
        if ($receipt.acknowledgement -ceq 'success') {
            try {
                $verified = Test-DesktopProbeReply $final.conversationState $CodexTestId $receipt.expected_test_marker $receipt.client_message_id
                $receipt.final_verification = $verified
                if ($false -cnotin $verified.Values -and $receipt.ack_turn_matches) { $valid++ }
            } catch {
                if ($_.Exception.GetBaseException().Message -cne 'test_turn_not_found') { throw }
                $receipt.missing_message_correlation = $true
            }
        } else {
            $matches = @(Get-DesktopProbeTurns $final.conversationState | Where-Object { $_.params.clientUserMessageId -ceq $receipt.client_message_id })
            $receipt.rejected_message_absent = $matches.Count -eq 0
            if ($receipt.rejected_message_absent) { $valid++ }
        }
    }
    $accepted = @($receipts | Where-Object { $_.acknowledgement -ceq 'success' })
    $report.accepted_requests = $accepted.Count
    $report.distinct_acknowledged_turns = @($accepted.ack_turn_id | Select-Object -Unique).Count
    $report.final_snapshot_verified = $valid -eq 2
    $report.test_execution_complete = $true
    $report.status = if ($report.final_snapshot_verified -and $accepted.Count -ge 1) { 'ok' } else { 'observed_failure' }
} catch {
    $report.failed_stage = $stage
    $report.failure_type = $_.Exception.GetType().Name
    $report.failure_id = $_.FullyQualifiedErrorId
    $report.failure_line = $_.InvocationInfo.ScriptLineNumber
    $reason = $_.Exception.GetBaseException().Message
    if ($reason -cin @('writer_binding_mismatch','test_thread_not_ready','test_write_already_attempted','duplicate_test_message',
        'response_owner_mismatch','test_turn_ack_unrecognized','codex_owner_required','owner_disconnected','stream_revision_gap')) {
        $report.failure_reason = $reason
    }
} finally {
    foreach ($session in $sessions) { Close-DesktopProbeSession $session }
    Save-Receipt
    $report | ConvertTo-Json -Depth 9
}
if ($report.status -cne 'ok') { exit 1 }
