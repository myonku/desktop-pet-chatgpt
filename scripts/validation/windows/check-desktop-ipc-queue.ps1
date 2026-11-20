#Requires -Version 7.4
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidatePattern('^[A-Za-z0-9_-]{1,128}\z')][string] $CodexTestId,
    [Parameter(Mandatory)][string] $OutputDirectory,
    [ValidateRange(10,120)][int] $TimeoutSeconds = 120,
    [switch] $TwoWriters,
    [string] $VerifyReceiptDirectory
)
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot desktop_ipc.psm1) -Force
if (Test-Path -LiteralPath (Join-Path $OutputDirectory result.json)) { throw 'queue_receipt_already_exists' }
$outputPath = (New-Item -ItemType Directory -Path $OutputDirectory -Force).FullName
$report = [ordered]@{ schema_version = 1; status = 'error'; probe_scope = 'owner_queue_reserve_then_complete'
    desktop_ui_verified = $false; queue_mutation_attempts = 0; direct_start_turn_attempts = 0; automatic_resends = 0
    queue_broadcasts = 0; requests = @() }
$session = $null
$secondSession = $null
$queueTrace = @{ latestOwnMessages = @() }
$stage = 'initial_snapshot'
function Save-Receipt { $report | ConvertTo-Json -Depth 9 | Set-Content -LiteralPath (Join-Path $outputPath result.json) }
function Observe-Queue($Event) {
    if ($Event.type -cne 'broadcast' -or $Event.method -cne 'thread-queued-followups-changed' -or
        $Event.sourceClientId -cne $session.owner -or $Event.params.hostId -cne 'local' -or
        $Event.params.conversationId -cne $CodexTestId -or
        ($null -ne $Event.targetClientIds -and $session.client -cnotin $Event.targetClientIds -and
            (-not $secondSession -or $secondSession.client -cnotin $Event.targetClientIds))) { return }
    if ($Event.version -cne 3 -or $Event.params.messages -isnot [array]) { throw 'unsupported_queue_state' }
    $report.queue_broadcasts++
    $ids = @($report.requests | ForEach-Object { $_.client_message_id })
    $queueTrace.latestOwnMessages = @($Event.params.messages | Where-Object { $_.id -cin $ids })
}
function Queue-Request($Request, [switch] $Reserve) {
    $parameters = New-DesktopProbeQueueRequest $initialState $CodexTestId $Request.expected_test_marker $Request.client_message_id -Reserve:$Reserve
    if ($Reserve) { $Request.reserve_attempted = $true } else { $Request.complete_attempted = $true }
    $report.queue_mutation_attempts++
    Save-Receipt
    $writer = if ($Request.writer_index -eq 1) { $secondSession } else { $session }
    $reply = Invoke-DesktopProbeRequest $writer.pipe thread-follower-set-queued-follow-ups-state $parameters $writer.client `
        -Target $writer.owner -AllowTestQueue -OnBroadcast ${function:Observe-Queue} -TimeoutMs 15000
    if ($reply.resultType -cne 'success' -or $reply.result.ok -cne $true) { throw 'queue_request_unconfirmed' }
    if ($Reserve) { $Request.reserve_acknowledged = $true } else { $Request.complete_acknowledged = $true }
    Save-Receipt
}
try {
    $session = Open-DesktopProbeSession $CodexTestId
    $report.desktop_version = $session.desktopVersion
    $report.desktop_peer_verified = $true
    $snapshot = Get-DesktopProbeSessionSnapshot $session
    $initialState = $snapshot.conversationState
    if ($VerifyReceiptDirectory) {
        if ($TwoWriters) { throw 'readback_cannot_write' }
        $saved = Get-Content -LiteralPath (Join-Path $VerifyReceiptDirectory result.json) -Raw | ConvertFrom-Json -AsHashtable
        if ($saved.probe_scope -cne 'owner_queue_reserve_then_complete' -or $saved.requests.Count -ne 2 -or
            $saved.queue_mutation_attempts -ne 4) { throw 'invalid_queue_receipt' }
        foreach ($request in $saved.requests) {
            if ($request.expected_test_marker -cnotmatch '^DESKTOP_PET_M6_QUEUE_[a-f0-9]{32}\z' -or
                $request.client_message_id -cnotmatch '^[a-f0-9-]{36}\z') { throw 'invalid_queue_receipt' }
            $report.requests += @{ expected_test_marker = $request.expected_test_marker; client_message_id = $request.client_message_id }
        }
        $report.probe_scope = 'owner_queue_history_readback'
    } else {
    Assert-DesktopProbeTestWriteReady $initialState
    if ($TwoWriters) {
        $secondSession = Open-DesktopProbeSession $CodexTestId -OwnerHint $session.owner
        $secondSnapshot = Get-DesktopProbeSessionSnapshot $secondSession
        Assert-DesktopProbeTestWriteReady $secondSnapshot.conversationState
        $report.independent_writer_clients = $session.client -cne $secondSession.client
        $report.same_owner_and_peer = $session.owner -ceq $secondSession.owner -and $session.peerGeneration -ceq $secondSession.peerGeneration
        if (-not $report.independent_writer_clients -or -not $report.same_owner_and_peer) { throw 'writer_binding_mismatch' }
    }
    foreach ($index in 0..1) {
        $report.requests += [ordered]@{ expected_test_marker = ('DESKTOP_PET_M6_QUEUE_' + [guid]::NewGuid().ToString('N'))
            client_message_id = [guid]::NewGuid().ToString(); reserve_attempted = $false; complete_attempted = $false
            writer_index = $(if ($TwoWriters) { $index } else { 0 }) }
    }
    $stage = 'reserve_both_without_replacing_queue'
    foreach ($request in $report.requests) { Queue-Request $request -Reserve }
    $report.incremental_preparation_ids_used = $true
    $reservedSnapshot = Get-DesktopProbeSessionSnapshot $session -OnQueue ${function:Observe-Queue}
    $report.reservations_did_not_start_turns = @((Get-DesktopProbeTurns $reservedSnapshot.conversationState) | Where-Object {
        $_.params.clientUserMessageId -cin @($report.requests.client_message_id)
    }).Count -eq 0
    if (-not $report.reservations_did_not_start_turns) { throw 'reserved_message_started_early' }
    $stage = 'complete_both_reserved_messages'
    if ($TwoWriters) {
        $timer = [Diagnostics.Stopwatch]::StartNew()
        foreach ($request in $report.requests) {
            $writer = if ($request.writer_index -eq 1) { $secondSession } else { $session }
            $request.complete_attempted = $true
            $request.completion_request_id = [guid]::NewGuid().ToString()
            $report.queue_mutation_attempts++
            Save-Receipt
            $parameters = New-DesktopProbeQueueRequest $initialState $CodexTestId $request.expected_test_marker $request.client_message_id
            Send-DesktopProbeQueueFrame $writer.pipe $parameters $writer.client $writer.owner -RequestId $request.completion_request_id -AllowTestQueue
            $request.completion_frame_written_ms = $timer.ElapsedMilliseconds
        }
        $report.completions_sent_before_any_ack_read = $true
        $report.completion_write_interval_ms = $report.requests[1].completion_frame_written_ms - $report.requests[0].completion_frame_written_ms
        Save-Receipt
        foreach ($request in $report.requests) {
            $writer = if ($request.writer_index -eq 1) { $secondSession } else { $session }
            $reply = Receive-DesktopProbeTestAcknowledgement $writer $request.completion_request_id `
                -ExpectedMethod thread-follower-set-queued-follow-ups-state -OnBroadcast ${function:Observe-Queue}
            if ($reply.resultType -cne 'success' -or $reply.result.ok -cne $true) { throw 'queue_request_unconfirmed' }
            $request.complete_acknowledged = $true
            Save-Receipt
        }
    } else {
    foreach ($request in $report.requests) {
        if ($request -eq $report.requests[1]) {
            $beforeSecond = Get-DesktopProbeSessionSnapshot $session -OnQueue ${function:Observe-Queue}
            $report.active_before_second_completion = $beforeSecond.conversationState.threadRuntimeStatus.type -ceq 'active'
        }
        Queue-Request $request
    }
    }
    Close-DesktopProbeSession $secondSession
    $secondSession = $null
    }
    $snapshot = Get-DesktopProbeSessionSnapshot $session -OnQueue ${function:Observe-Queue}
    $current = @{ revision = $snapshot.revision; conversationState = $snapshot.conversationState }
    $stage = 'observe_owner_queue_execution'
    $deadline = [Threading.CancellationTokenSource]::new($TimeoutSeconds * 1000)
    try {
        while ($true) {
            $completed = 0
            foreach ($request in $report.requests) {
                $matches = @(Get-DesktopProbeTurns $current.conversationState | Where-Object {
                    $_.params.clientUserMessageId -ceq $request.client_message_id
                })
                $request.matching_turns = $matches.Count
                if ($matches.Count -gt 1) { throw 'duplicate_test_message' }
                if ($matches.Count -eq 1) {
                    $request.turn_status = $matches[0].status
                    $request.turn_id = $matches[0].turnId
                    if ($matches[0].status -ceq 'completed') {
                        $request.verification = Test-DesktopProbeReply $current.conversationState $CodexTestId `
                            $request.expected_test_marker $request.client_message_id
                        if ($false -cin $request.verification.Values) { throw 'queued_reply_mismatched' }
                        $completed++
                    }
                    if ($matches[0].status -cin @('failed','interrupted')) { throw 'queued_turn_failed' }
                }
            }
            Save-Receipt
            if ($completed -eq 2 -and $current.conversationState.threadRuntimeStatus.type -ceq 'idle') { break }
            $change = Receive-DesktopProbeSessionChange $session $deadline.Token -OnQueue ${function:Observe-Queue}
            $null = Update-DesktopProbeStream $current $change $CodexTestId
        }
    } finally { $deadline.Dispose() }
    $final = Get-DesktopProbeSessionSnapshot $session -OnQueue ${function:Observe-Queue}
    foreach ($request in $report.requests) {
        $request.final_verification = Test-DesktopProbeReply $final.conversationState $CodexTestId `
            $request.expected_test_marker $request.client_message_id
        if ($false -cin $request.final_verification.Values) { throw 'queued_reply_mismatched' }
    }
    $report.distinct_completed_turns = @($report.requests.turn_id | Select-Object -Unique).Count
    $report.no_inprogress_rows = @(Get-DesktopProbeTurns $final.conversationState | Where-Object { $_.status -ceq 'inProgress' }).Count -eq 0
    $report.no_unconfirmed_submissions = @($final.conversationState.unconfirmedTurnSubmissions).Where({ $null -ne $_ }).Count -eq 0
    $report.queue_presence_verified = $report.queue_broadcasts -ge 1
    $report.own_queue_remaining = if ($report.queue_presence_verified) { $queueTrace.latestOwnMessages.Count } else { $null }
    if ($report.distinct_completed_turns -ne 2 -or -not $report.no_inprogress_rows -or
        -not $report.no_unconfirmed_submissions -or (-not $VerifyReceiptDirectory -and
        (-not $report.queue_presence_verified -or $report.own_queue_remaining -ne 0))) { throw 'queue_not_converged' }
    $report.final_snapshot_verified = $true
    $report.status = 'ok'
} catch {
    $report.failed_stage = $stage
    $report.failure_type = $_.Exception.GetType().Name
    $report.failure_id = $_.FullyQualifiedErrorId
    $report.failure_line = $_.InvocationInfo.ScriptLineNumber
    $reason = $_.Exception.GetBaseException().Message
    if ($reason -cin @('codex_owner_required','unsupported_desktop_package','test_thread_not_ready','test_write_already_attempted',
        'queue_request_unconfirmed','reserved_message_started_early','duplicate_test_message','queued_reply_mismatched',
        'queued_turn_failed','queue_not_converged','unsupported_queue_state','unsupported_queue_version','owner_disconnected','stream_revision_gap')) {
        $report.failure_reason = $reason
    }
} finally {
    # Only this experiment's acknowledged reservations that were never committed
    # may be removed automatically. Committed/unconfirmed sends are never replayed.
    foreach ($request in $report.requests) {
        if (-not $VerifyReceiptDirectory -and $session -and $request.reserve_acknowledged -and -not $request.complete_attempted) {
            try {
                $reply = Invoke-DesktopProbeRequest $session.pipe thread-follower-remove-queued-message @{
                    conversationId = $CodexTestId; messageId = $request.client_message_id
                } $session.client -Target $session.owner -AllowTestQueueRemoval -OnBroadcast ${function:Observe-Queue}
                $request.uncommitted_reservation_removed = $reply.resultType -ceq 'success' -and $null -ne $reply.result.removed
            } catch { $request.uncommitted_reservation_cleanup_unconfirmed = $true }
        }
    }
    Close-DesktopProbeSession $session
    Close-DesktopProbeSession $secondSession
    Save-Receipt
    $report | ConvertTo-Json -Depth 9
}
if ($report.status -cne 'ok') { exit 1 }
