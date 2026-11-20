#Requires -Version 7.4
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidatePattern('^[A-Za-z0-9_-]{1,128}\z')][string] $CodexTestId,
    [Parameter(Mandatory)][string] $OutputDirectory,
    [ValidateRange(10,120)][int] $TimeoutSeconds = 35,
    [switch] $DesktopInterleave,
    [string] $ReadbackReceipt,
    [switch] $PrepareRestart,
    [string] $RestartReceipt,
    [string] $PreviousLifecycleReceipt
)

$ErrorActionPreference = 'Stop'
if (@(@($DesktopInterleave, $ReadbackReceipt, $PrepareRestart, $RestartReceipt) | Where-Object { $_ }).Count -gt 1) {
    throw 'choose_one_queue_lifecycle_mode'
}
if ($PreviousLifecycleReceipt -and ($DesktopInterleave -or $ReadbackReceipt -or $PrepareRestart -or $RestartReceipt)) {
    throw 'previous_receipt_requires_ack_loss_experiment'
}
Import-Module (Join-Path $PSScriptRoot desktop_ipc.psm1) -Force
$outputPath = (New-Item -ItemType Directory -Path $OutputDirectory -Force).FullName
$receiptPath = Join-Path $outputPath result.json
# Reserve the receipt before opening a connection. An existing receipt may
# describe a request whose outcome is still unknown.
$receipt = [IO.File]::Open($receiptPath, [IO.FileMode]::CreateNew)
$receipt.Dispose()

$report = [ordered]@{
    schema_version = 1; status = 'error'; desktop_ui_verified = $false
    probe_scope = $(if ($DesktopInterleave) { 'desktop_queue_interleave' } else { 'queue_ack_loss_and_cancellation' })
    queue_mutation_attempts = 0; direct_start_turn_attempts = 0; automatic_resends = 0
    selected_queue_broadcasts = 0
}
$observer = $null
$writer = $null
$stage = 'initial_snapshot'
$queueTrace = @{ items = @() }
$testMessages = @()

function Save-Receipt {
    $report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $receiptPath
}

function New-TestMessage {
    return [ordered]@{
        id = [guid]::NewGuid().ToString()
        marker = 'DESKTOP_PET_M6_QUEUE_' + [guid]::NewGuid().ToString('N')
    }
}

function Observe-Queue($Event) {
    if ($Event.type -cne 'broadcast' -or $Event.method -cne 'thread-queued-followups-changed' -or
        $Event.sourceClientId -cne $observer.owner -or $Event.params.hostId -cne 'local' -or
        $Event.params.conversationId -cne $CodexTestId -or
        ($null -ne $Event.targetClientIds -and $observer.client -cnotin $Event.targetClientIds)) { return }
    if ($Event.version -cne 3 -or $Event.params.messages -isnot [array]) { throw 'unsupported_queue_state' }
    $report.selected_queue_broadcasts++
    $queueTrace.items = @($Event.params.messages | Where-Object { $_.id -cin $testMessages.id } |
        ForEach-Object { @{ id = $_.id; prepared = $null -ne $_.preparation } })
    $report.last_broadcast_own_queue_count = $queueTrace.items.Count
}

function Wait-QueueState([scriptblock] $Condition) {
    $deadline = [Threading.CancellationTokenSource]::new($TimeoutSeconds * 1000)
    try {
        while (-not (& $Condition)) {
            $event = Read-DesktopProbeFrame $observer.pipe $deadline.Token
            if (Invoke-DesktopProbeControl $observer.pipe $event $deadline.Token) { continue }
            if (($event.method -ceq 'client-status-changed' -and $event.params.clientId -ceq $observer.owner -and
                $event.params.status -ceq 'disconnected') -or $event.method -ceq 'ipc-connection-reset') { throw 'owner_disconnected' }
            Observe-Queue $event
        }
    } finally { $deadline.Dispose() }
}

function Open-Writer {
    $connection = Open-DesktopProbeSession $CodexTestId -OwnerHint $observer.owner
    if ($connection.peerGeneration -cne $observer.peerGeneration -or $connection.owner -cne $observer.owner -or
        $connection.client -ceq $observer.client) {
        Close-DesktopProbeSession $connection
        throw 'writer_binding_mismatch'
    }
    return $connection
}

function Submit-Queue($Message, [string] $Operation, [switch] $DropAck) {
    $requestId = [guid]::NewGuid()
    $parameters = New-DesktopProbeQueueRequest $initial.conversationState $CodexTestId $Message.marker $Message.id `
        -Reserve:($Operation -ceq 'reserve')
    $Message[$Operation] = [ordered]@{ request_id = $requestId.ToString(); frame_written = $false; acknowledgement_read = $false }
    $report.queue_mutation_attempts++
    Save-Receipt
    Send-DesktopProbeQueueFrame $writer.pipe $parameters $writer.client $writer.owner -RequestId $requestId -AllowTestQueue
    $Message[$Operation].frame_written = $true
    if (-not $DropAck) {
        $reply = Receive-DesktopProbeTestAcknowledgement $writer $requestId -ExpectedMethod thread-follower-set-queued-follow-ups-state
        $Message[$Operation].acknowledgement_read = $true
        if ($reply.resultType -cne 'success' -or $reply.result.ok -cne $true) { throw 'queue_request_unconfirmed' }
        $Message[$Operation].acknowledged = $true
    }
    Save-Receipt
}

function Get-CorrelatedTurns($State, $Message) {
    Get-DesktopProbeTurns $State | Where-Object {
        $_.params.clientUserMessageId -ceq $Message.id -or
        @($_.items | Where-Object {
            $_.type -ceq 'userMessage' -and ($_.clientId -ceq $Message.id -or
                ((@($_.content | ForEach-Object { $_.text }) -join '').Contains($Message.marker)))
        }).Count -gt 0
    }
}

function Get-Fingerprint([string] $Value) {
    return [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($Value)))
}

function Assert-SettledHistory($State) {
    if ($State.turnHistory.history.isComplete -cne $true -or $State.threadRuntimeStatus.type -cne 'idle' -or
        @(Get-DesktopProbeTurns $State | Where-Object { $_.status -ceq 'inProgress' }).Count -or
        @($State.unconfirmedTurnSubmissions | Where-Object { $null -ne $_ }).Count) { throw 'test_thread_not_ready' }
}

try {
    $observer = Open-DesktopProbeSession $CodexTestId
    $report.desktop_version = $observer.desktopVersion
    $initial = Get-DesktopProbeSessionSnapshot $observer
    $initialTurns = @(Get-DesktopProbeTurns $initial.conversationState)
    $report.conversation_fingerprint = Get-Fingerprint $CodexTestId
    if ($PrepareRestart) {
        Assert-SettledHistory $initial.conversationState
        $report.probe_scope = 'queue_restart_preparation'
        $report.peer_generation_fingerprint = Get-Fingerprint $observer.peerGeneration
        $restartMessage = New-TestMessage
        $testMessages += $restartMessage
        $report.restart_message = $restartMessage
        $writer = Open-Writer
        $stage = 'prepare_reservation_for_restart'
        Submit-Queue $restartMessage reserve
        Wait-QueueState { @($queueTrace.items | Where-Object { $_.id -ceq $restartMessage.id -and $_.prepared }).Count -eq 1 }
        $beforeRestart = (Get-DesktopProbeSessionSnapshot $observer -OnQueue ${function:Observe-Queue}).conversationState
        $report.reservation_observed = $true
        $report.absent_from_history = @(Get-CorrelatedTurns $beforeRestart $restartMessage).Count -eq 0
        if (-not $report.absent_from_history) { throw 'reserved_message_started_early' }
        $report.status = 'ok'
        return
    }
    if ($RestartReceipt) {
        $report.probe_scope = 'queue_restart_readback_and_cancel'
        $saved = Get-Content -LiteralPath $RestartReceipt -Raw | ConvertFrom-Json -AsHashtable
        $restartMessage = $saved.restart_message
        if ($saved.probe_scope -cne 'queue_restart_preparation' -or $saved.status -cne 'ok' -or
            $saved.conversation_fingerprint -cne $report.conversation_fingerprint -or
            $saved.peer_generation_fingerprint -cnotmatch '^[A-F0-9]{64}\z' -or $saved.reservation_observed -cne $true -or
            $restartMessage.reserve.acknowledged -cne $true -or $restartMessage.id -cnotmatch '^[a-f0-9-]{36}\z' -or
            $restartMessage.marker -cnotmatch '^DESKTOP_PET_M6_QUEUE_[a-f0-9]{32}\z' -or $restartMessage.Contains('complete')) {
            throw 'invalid_restart_receipt'
        }
        $report.desktop_process_changed = (Get-Fingerprint $observer.peerGeneration) -cne $saved.peer_generation_fingerprint
        if (-not $report.desktop_process_changed) { throw 'desktop_process_has_not_restarted' }
        Assert-SettledHistory $initial.conversationState
        if (@(Get-CorrelatedTurns $initial.conversationState $restartMessage).Count) { throw 'reserved_message_started_after_restart' }
        $testMessages += $restartMessage
        $stage = 'cancel_pre_restart_reservation'
        $report.queue_mutation_attempts++
        Save-Receipt
        $reply = Invoke-DesktopProbeRequest $observer.pipe thread-follower-remove-queued-message @{
            conversationId = $CodexTestId; messageId = $restartMessage.id
        } $observer.client -Target $observer.owner -AllowTestQueueRemoval -OnBroadcast ${function:Observe-Queue}
        if ($reply.resultType -cne 'success' -or -not $reply.result.Contains('removed')) { throw 'cancel_outcome_unconfirmed' }
        $report.reservation_survived_restart = $null -ne $reply.result.removed
        if ($report.reservation_survived_restart -and ($reply.result.removed.message.id -cne $restartMessage.id -or
            $null -eq $reply.result.removed.message.preparation)) { throw 'removed_reservation_mismatch' }
        $report.cancel_acknowledged = $true
        $final = (Get-DesktopProbeSessionSnapshot $observer -OnQueue ${function:Observe-Queue}).conversationState
        $report.absent_from_history_after_restart = @(Get-CorrelatedTurns $final $restartMessage).Count -eq 0
        $report.queue_presence_verified = $report.selected_queue_broadcasts -gt 0
        $report.own_queue_remaining = if ($report.queue_presence_verified) { $queueTrace.items.Count } else { $null }
        if (-not $report.absent_from_history_after_restart -or $report.own_queue_remaining -gt 0) { throw 'queue_not_converged' }
        $report.status = 'ok'
        return
    }
    if ($ReadbackReceipt) {
        $saved = Get-Content -LiteralPath $ReadbackReceipt -Raw | ConvertFrom-Json -AsHashtable
        if ($saved.probe_scope -cne 'queue_ack_loss_and_cancellation') { throw 'invalid_lifecycle_receipt' }
        foreach ($message in @($saved.cancelled_message, $saved.delivered_message)) {
            if ($message.id -cnotmatch '^[a-f0-9-]{36}\z' -or
                $message.marker -cnotmatch '^DESKTOP_PET_M6_QUEUE_[a-f0-9]{32}\z') { throw 'invalid_lifecycle_receipt' }
        }
        $report.probe_scope = 'queue_lifecycle_history_readback'
        $report.cancelled_message_absent_from_history = @(Get-CorrelatedTurns $initial.conversationState $saved.cancelled_message).Count -eq 0
        $delivered = @(Get-CorrelatedTurns $initial.conversationState $saved.delivered_message)
        $report.delivered_matching_turns = $delivered.Count
        if ($delivered.Count -eq 1) {
            $report.delivered_status = $delivered[0].status
            $report.delivered_item_types = @($delivered[0].items | ForEach-Object { $_.type })
            $report.retrying_error_items = @($delivered[0].items | Where-Object { $_.type -ceq 'error' -and $_.willRetry -eq $true }).Count
            $report.delivered_verification = Test-DesktopProbeReply $initial.conversationState $CodexTestId `
                $saved.delivered_message.marker $saved.delivered_message.id
        }
        $deliveryChecks = @('completed', 'reply_matches', 'user_message_matches', 'client_message_id_matches', 'thread_id_matches')
        $report.delivery_confirmed = $delivered.Count -eq 1 -and
            @($deliveryChecks | Where-Object { -not $report.delivered_verification[$_] }).Count -eq 0
        $report.clean_completion = $report.delivery_confirmed -and $report.delivered_verification.no_tool_items
        $report.runtime_status = $initial.conversationState.threadRuntimeStatus.type
        $report.inprogress_rows = @($initialTurns | Where-Object { $_.status -ceq 'inProgress' }).Count
        $report.queue_presence_verified = $false
        $report.status = 'ok'
        return
    }
    if ($DesktopInterleave) {
        # The same dedicated session may contain completed lifecycle test messages.
        Assert-SettledHistory $initial.conversationState
        $clientMarker = 'DESKTOP_PET_M6_CLIENT_INTERLEAVE_' + [guid]::NewGuid().ToString('N').Substring(0,8).ToUpperInvariant()
        $report.desktop_marker = $clientMarker
    } else {
        if ($PreviousLifecycleReceipt) {
            Assert-SettledHistory $initial.conversationState
            $previous = Get-Content -LiteralPath $PreviousLifecycleReceipt -Raw | ConvertFrom-Json -AsHashtable
            if ($previous.probe_scope -cne 'queue_ack_loss_and_cancellation' -or $previous.cancel_acknowledged -cne $true -or
                $previous.delivered_message.complete.frame_written -cne $true) { throw 'invalid_lifecycle_receipt' }
            foreach ($message in @($previous.cancelled_message, $previous.delivered_message)) {
                if ($message.id -cnotmatch '^[a-f0-9-]{36}\z' -or
                    $message.marker -cnotmatch '^DESKTOP_PET_M6_QUEUE_[a-f0-9]{32}\z') { throw 'invalid_lifecycle_receipt' }
            }
            $verified = Test-DesktopProbeReply $initial.conversationState $CodexTestId `
                $previous.delivered_message.marker $previous.delivered_message.id
            if (@(Get-CorrelatedTurns $initial.conversationState $previous.cancelled_message).Count -or
                @($verified.Keys | Where-Object { $_ -cne 'no_tool_items' -and -not $verified[$_] }).Count) {
                throw 'previous_lifecycle_delivery_not_settled'
            }
            $report.previous_delivery_reverified = $true
        } else {
            Assert-DesktopProbeTestWriteReady $initial.conversationState
        }
        $cancelled = New-TestMessage
        $testMessages += $cancelled
        $report.cancelled_message = $cancelled
    }
    $delivered = New-TestMessage
    $testMessages += $delivered
    $report.delivered_message = $delivered
    $writer = Open-Writer

    if ($DesktopInterleave) {
        Save-Receipt
        $stage = 'wait_for_desktop_turn'
        Write-Host "READY: $clientMarker"
        $current = @{ revision = $initial.revision; conversationState = $initial.conversationState }
        $deadline = [Threading.CancellationTokenSource]::new($TimeoutSeconds * 1000)
        try {
            do {
                $change = Receive-DesktopProbeSessionChange $observer $deadline.Token -OnQueue ${function:Observe-Queue}
                $null = Update-DesktopProbeStream $current $change $CodexTestId
                $desktopTurns = @(Get-DesktopProbeTurns $current.conversationState | Where-Object {
                    $_.turnId -and @($_.params.input | Where-Object {
                        $_.type -ceq 'text' -and [string]$_.text -clike "$clientMarker*"
                    }).Count -gt 0
                })
            } until ($desktopTurns.Count -gt 0)
        } finally { $deadline.Dispose() }
        if ($desktopTurns.Count -ne 1 -or $desktopTurns[0].status -cne 'inProgress') { throw 'desktop_turn_not_active' }
        $report.desktop_turn_id = $desktopTurns[0].turnId
        $report.desktop_active_before_reservation = $true
        $stage = 'submit_during_desktop_turn'
        Submit-Queue $delivered reserve
        $beforeComplete = Get-DesktopProbeSessionSnapshot $observer -OnQueue ${function:Observe-Queue}
        $active = @(Get-DesktopProbeTurns $beforeComplete.conversationState | Where-Object {
            $_.turnId -ceq $report.desktop_turn_id -and $_.status -ceq 'inProgress'
        })
        if ($active.Count -ne 1) { throw 'desktop_turn_ended_before_queue_completion' }
        $report.desktop_active_before_completion = $true
        Submit-Queue $delivered complete
    } else {
        $stage = 'reserve_without_reading_ack'
        Submit-Queue $cancelled reserve -DropAck
        Close-DesktopProbeSession $writer -Abrupt
        $previousWriter = $writer.client
        $writer = $null
        $report.reserve_writer_disconnected = $true
        Save-Receipt

        $stage = 'reconnect_and_find_reservation'
        $writer = Open-Writer
        $report.reserve_writer_reconnected = $writer.client -cne $previousWriter
        Wait-QueueState { @($queueTrace.items | Where-Object { $_.id -ceq $cancelled.id -and $_.prepared }).Count -eq 1 }
        $report.original_reservation_observed = $true
        Submit-Queue $delivered reserve
        Wait-QueueState { $queueTrace.items.Count -eq 2 }

        $stage = 'cancel_only_original_reservation'
        $report.queue_mutation_attempts++
        $report.cancel_attempted = $true
        Save-Receipt
        $reply = Invoke-DesktopProbeRequest $writer.pipe thread-follower-remove-queued-message @{
            conversationId = $CodexTestId; messageId = $cancelled.id
        } $writer.client -Target $writer.owner -AllowTestQueueRemoval
        $report.cancel_acknowledged = $reply.resultType -ceq 'success' -and $reply.result.removed.message.id -ceq $cancelled.id
        if (-not $report.cancel_acknowledged) { throw 'cancel_outcome_unconfirmed' }
        Wait-QueueState {
            $queueTrace.items.Count -eq 1 -and $queueTrace.items[0].id -ceq $delivered.id -and $queueTrace.items[0].prepared
        }
        $report.other_reservation_preserved = $true

        $stage = 'complete_without_reading_ack'
        Submit-Queue $delivered complete -DropAck
        Close-DesktopProbeSession $writer -Abrupt
        $previousWriter = $writer.client
        $writer = $null
        $report.complete_writer_disconnected = $true
        Save-Receipt
        $writer = Open-Writer
        $report.complete_writer_reconnected = $writer.client -cne $previousWriter
        $null = Get-DesktopProbeSessionSnapshot $writer
        $report.fresh_snapshot_after_reconnect = $true
        Close-DesktopProbeSession $writer
        $writer = $null
    }

    $stage = 'verify_final_history_and_queue'
    $snapshot = Get-DesktopProbeSessionSnapshot $observer -OnQueue ${function:Observe-Queue}
    $current = @{ revision = $snapshot.revision; conversationState = $snapshot.conversationState }
    $timer = [Diagnostics.Stopwatch]::StartNew()
    while ($true) {
        $matches = @(Get-CorrelatedTurns $current.conversationState $delivered)
        $report.observed_delivery_count = $matches.Count
        $report.observed_delivery_status = if ($matches.Count -eq 1) { $matches[0].status } else { $null }
        if ($matches.Count -gt 1) { throw 'duplicate_queued_message' }
        if ($matches.Count -eq 1 -and $matches[0].status -cin @('failed','interrupted')) { throw 'queued_turn_failed' }
        if ($matches.Count -eq 1 -and $matches[0].status -ceq 'completed' -and
            $current.conversationState.threadRuntimeStatus.type -ceq 'idle') { break }
        if ($timer.Elapsed.TotalSeconds -ge $TimeoutSeconds) { throw 'queue_completion_deadline' }
        Start-Sleep -Milliseconds 400
        $current.conversationState = (Get-DesktopProbeSessionSnapshot $observer -OnQueue ${function:Observe-Queue}).conversationState
    }
    $stage = 'verify_queue_clear_broadcast'
    Wait-QueueState { $report.selected_queue_broadcasts -gt 0 -and $queueTrace.items.Count -eq 0 }
    $final = (Get-DesktopProbeSessionSnapshot $observer -OnQueue ${function:Observe-Queue}).conversationState
    $report.delivered_verification = Test-DesktopProbeReply $final $CodexTestId $delivered.marker $delivered.id
    $report.delivered_item_types = @((Get-CorrelatedTurns $final $delivered).items | ForEach-Object { $_.type })
    $report.final_matching_turns = @(Get-CorrelatedTurns $final $delivered).Count
    $report.no_inprogress_rows = @(Get-DesktopProbeTurns $final | Where-Object { $_.status -ceq 'inProgress' }).Count -eq 0
    $report.no_unconfirmed_submissions = @($final.unconfirmedTurnSubmissions | Where-Object { $null -ne $_ }).Count -eq 0
    $report.own_queue_remaining = $queueTrace.items.Count
    if ($DesktopInterleave) {
        $desktop = @(Get-DesktopProbeTurns $final | Where-Object { $_.turnId -ceq $report.desktop_turn_id })
        $report.desktop_turn_completed = $desktop.Count -eq 1 -and $desktop[0].status -ceq 'completed'
        $report.desktop_reply_has_marker = @($desktop.items | Where-Object {
            $_.type -ceq 'agentMessage' -and [string]$_.text -clike "*$clientMarker*"
        }).Count -eq 1
        $report.distinct_turns = $matches[0].turnId -cne $report.desktop_turn_id
        $report.desktop_turn_has_one_user = @($desktop.items | Where-Object { $_.type -ceq 'userMessage' }).Count -eq 1
        if (-not ($report.desktop_turn_completed -and $report.desktop_reply_has_marker -and
            $report.distinct_turns -and $report.desktop_turn_has_one_user)) { throw 'desktop_reply_not_isolated' }
    } else {
        $report.cancelled_message_absent_from_history = @(Get-CorrelatedTurns $final $cancelled).Count -eq 0
        if (-not $report.cancelled_message_absent_from_history -or -not $report.reserve_writer_reconnected -or
            -not $report.complete_writer_reconnected) { throw 'queue_recovery_unverified' }
    }
    if ($false -cin $report.delivered_verification.Values -or $report.final_matching_turns -ne 1 -or
        -not $report.no_inprogress_rows -or -not $report.no_unconfirmed_submissions -or $report.own_queue_remaining -ne 0) {
        throw 'queue_not_converged'
    }
    $report.status = 'ok'
} catch {
    $report.failed_stage = $stage
    $report.failure_id = $_.FullyQualifiedErrorId
    $report.failure_line = $_.InvocationInfo.ScriptLineNumber
} finally {
    Close-DesktopProbeSession $writer
    Close-DesktopProbeSession $observer
    Save-Receipt
    $report | ConvertTo-Json -Depth 8
}
if ($report.status -cne 'ok') { exit 1 }
