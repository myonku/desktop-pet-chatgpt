#Requires -Version 7.4
<#
.SYNOPSIS
Check whether the running packaged ChatGPT desktop client can route a test ID
through private Codex IPC. Default: discovery only. Snapshot and fixed test turn
experiments require explicit switches and an authorized Codex test ID.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string] $OutputDirectory,
    [ValidatePattern('^[A-Za-z0-9_-]{1,128}\z')][string] $ChatTestId,
    [ValidatePattern('^[A-Za-z0-9_-]{1,128}\z')][string] $CodexTestId,
    [ValidatePattern('^[A-Za-z0-9_-]{1,128}\z')][string] $WorkTestId,
    [switch] $CodexSnapshot,
    [ValidateRange(1, 100)][int] $SnapshotVersion = 11,
    [switch] $CodexTestTurn,
    [ValidatePattern('^DESKTOP_PET_M6_IPC_[a-f0-9]{32}\z')][string] $ExpectedTestMarker,
    [ValidatePattern('^[a-f0-9-]{36}\z')][string] $ExpectedMessageId,
    [switch] $VerifyClientTurn,
    [switch] $ObserveClientTurn,
    [switch] $StopClientTestTurn,
    [ValidateRange(5, 120)][int] $ObserveSeconds = 60,
    [ValidatePattern('^DESKTOP_PET_M6_CLIENT_[A-Z0-9_]{2,64}\z')][string] $ClientTestMarker = 'DESKTOP_PET_M6_CLIENT_OK'
)

$ErrorActionPreference = 'Stop'
if (($ObserveClientTurn -or $StopClientTestTurn) -and ($CodexTestTurn -or $VerifyClientTurn -or $ExpectedMessageId)) { throw 'observe client turn separately' }
if ($ObserveClientTurn -and $StopClientTestTurn) { throw 'stop and reply observation are separate experiments' }
if (($ObserveClientTurn -or $StopClientTestTurn) -and $SnapshotVersion -ne 13) { throw 'observation_requires_v13' }
if ($StopClientTestTurn -and $ClientTestMarker -cnotmatch '^DESKTOP_PET_M6_CLIENT_STOP_[A-Z0-9_]+\z') { throw 'invalid_stop_marker' }
if ($CodexTestTurn -or $VerifyClientTurn -or $ObserveClientTurn -or $StopClientTestTurn) { $CodexSnapshot = $true }
if ($CodexTestTurn -and $VerifyClientTurn) { throw 'verify client turn without sending a new turn' }
if ($ExpectedTestMarker -or $ExpectedMessageId) {
    if (-not $ExpectedTestMarker -or -not $ExpectedMessageId -or $CodexTestTurn) { throw 'provide both verification fields without a new turn' }
    $CodexSnapshot = $true
}
if ($CodexSnapshot -and -not $CodexTestId) { throw 'CodexSnapshot requires CodexTestId' }
Import-Module (Join-Path $PSScriptRoot 'desktop_ipc.psm1') -Force
$outputPath = (New-Item -ItemType Directory -Path $OutputDirectory).FullName
$scope = if ($CodexTestTurn) { 'fixed_test_turn' } elseif ($StopClientTestTurn) { 'client_test_interrupt' }
    elseif ($ObserveClientTurn) { 'client_stream_observation' }
    elseif ($VerifyClientTurn) { 'client_reverse_verification' }
    elseif ($ExpectedMessageId) { 'test_reply_verification' }
    elseif ($CodexSnapshot) { 'snapshot' } else { 'owner_discovery' }
$report = [ordered]@{ schema_version = 1; status = 'error'; probe_scope = $scope; desktop_ui_verified = $false;
    desktop_version = $null; desktop_peer_verified = $false;
    codex_scheme_in_manifest = $false; initialize_v0 = $false; owner_discovery_v1 = [ordered]@{} }
$pipe = $null
$followingOwner = $null
$stage = 'package_discovery'

try {
    $packages = @(Get-AppxPackage OpenAI.Codex)
    if ($packages.Count -ne 1 -or $packages[0].PackageFamilyName -cne 'OpenAI.Codex_2p2nqsd0c76g0') {
        throw 'desktop_package_unavailable'
    }
    $app = $packages[0]
    $report.desktop_version = $app.Version.ToString()
    $manifest = Get-AppxPackageManifest -Package $app.PackageFullName
    $report.codex_scheme_in_manifest = $null -ne $manifest.SelectSingleNode('//*[local-name()="Protocol" and @Name="codex"]')
    $manifestApp = $manifest.SelectSingleNode('//*[local-name()="Application" and @Id="App"]')
    if (-not $manifestApp.Executable) { throw 'desktop_executable_missing' }
    $image = [IO.Path]::GetFullPath((Join-Path $app.InstallLocation $manifestApp.Executable))

    $stage = 'pipe_connect'
    Add-Type -TypeDefinition @'
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;
public static class M6DesktopPipe {
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool GetNamedPipeServerProcessId(SafePipeHandle pipe, out int pid);
}
'@
    $pipe = [IO.Pipes.NamedPipeClientStream]::new('.', 'codex-ipc',
        [IO.Pipes.PipeDirection]::InOut, [IO.Pipes.PipeOptions]::Asynchronous,
        [Security.Principal.TokenImpersonationLevel]::Identification)
    $pipe.Connect(2000)
    $stage = 'verify_pipe_peer'
    [int]$serverProcessId = 0
    if (-not [M6DesktopPipe]::GetNamedPipeServerProcessId($pipe.SafePipeHandle, [ref]$serverProcessId)) {
        throw 'pipe_peer_unknown'
    }
    $server = Get-Process -Id $serverProcessId
    if ($server.SessionId -ne (Get-Process -Id $PID).SessionId -or
        -not [string]::Equals($server.Path, $image, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'unexpected_pipe_peer'
    }
    $report.desktop_peer_verified = $true

    $stage = 'initialize_v0'
    $reply = Invoke-DesktopProbeRequest $pipe 'initialize' @{ clientType = 'bailong' } 'initializing-client'
    $clientId = $reply.result.clientId
    if ($reply.resultType -cne 'success' -or $reply.method -cne 'initialize' -or
        $clientId -isnot [string] -or $clientId -cnotmatch '^[A-Za-z0-9][A-Za-z0-9._:-]{0,199}\z') {
        throw 'initialize_not_confirmed'
    }
    $report.initialize_v0 = $true

    $stage = 'owner_discovery_v1'
    $ids = [ordered]@{ nonexistent = [guid]::NewGuid().ToString(); chat = $ChatTestId;
        codex = $CodexTestId; work = $WorkTestId }
    foreach ($kind in $ids.Keys) {
        if (-not $ids[$kind]) { continue }
        $reply = Invoke-DesktopProbeRequest $pipe 'thread-owner-discovery' @{
            hostId = 'local'; conversationId = $ids[$kind]
        } $clientId
        $report.owner_discovery_v1[$kind] = if ($reply.resultType -ceq 'success' -and
            $reply.method -ceq 'thread-owner-discovery' -and $reply.handledByClientId -is [string] -and
            $reply.handledByClientId -cmatch '^[A-Za-z0-9][A-Za-z0-9._:-]{0,199}\z') {
            'owner_found'
        } elseif ($reply.resultType -ceq 'error' -and $reply.error -ceq 'no-client-found') {
            'no_owner_found'
        } else { 'unrecognized_reply' }
        if ($kind -ceq 'codex' -and $CodexSnapshot) {
            if ($report.owner_discovery_v1[$kind] -cne 'owner_found') { throw 'codex_owner_required' }
            $followingOwner = $reply.handledByClientId
        }
    }
    if ($report.owner_discovery_v1.nonexistent -cne 'no_owner_found' -or
        'unrecognized_reply' -cin $report.owner_discovery_v1.Values) { throw 'unexpected_discovery' }
    if ($CodexSnapshot) {
        $stage = 'codex_snapshot'
        $report.expected_stream_version = $SnapshotVersion
        Send-DesktopProbeFollowing $pipe $clientId $followingOwner $CodexTestId $true
        $snapshot = Receive-DesktopProbeSnapshot $pipe $clientId $followingOwner $CodexTestId -ExpectedVersion $SnapshotVersion
        $state = $snapshot.conversationState
        if ($state -isnot [System.Collections.IDictionary] -or $state.id -cne $CodexTestId -or
            $state.hostId -cne 'local') { throw 'snapshot_identity_mismatch' }
        $entities = @(Get-DesktopProbeTurns $state)
        $report.codex_snapshot = @{ received = $true; route_matches = $true;
            state_id_matches = $true; history_turn_count = $entities.Count }
        if ($ExpectedMessageId) {
            $report.codex_test_verification = Test-DesktopProbeReply $state $CodexTestId $ExpectedTestMarker $ExpectedMessageId
            if ($false -cin $report.codex_test_verification.Values) { throw 'test_reply_unverified' }
        }
        if ($VerifyClientTurn) {
            $report.codex_client_reverse = Test-DesktopProbeClientReply $state $CodexTestId $ClientTestMarker
            if (-not $report.codex_client_reverse.reply_matches -or -not $report.codex_client_reverse.completed -or
                -not $report.codex_client_reverse.no_tool_items) { throw 'client_reply_unverified' }
        }
        if ($ObserveClientTurn -or $StopClientTestTurn) {
            $stage = 'codex_client_stream'
            if (@(Get-DesktopProbeTurns $state | Where-Object {
                @($_.params.input | Where-Object { $_.type -ceq 'text' -and $_.text.Contains($ClientTestMarker) }).Count -gt 0
            }).Count) { throw 'client_test_already_present' }
            $current = @{ revision = $snapshot.revision; conversationState = $state }
            $stats = [ordered]@{ patch_events = 0; snapshot_events = 0; stale_events = 0; text_updates = 0;
                partial_text_updates = 0; completed_from_patches = $false; final_snapshot_matches = $false; omitted_patch_values = 0 }
            $report.codex_client_stream = $stats
            if ($StopClientTestTurn) { Write-Host 'READY: waiting for the marked active client test turn; one targeted interrupt will be sent.' }
            else { Write-Host 'READY: observing client test turn; no message is sent by this probe.' }
            $deadline = [Threading.CancellationTokenSource]::new($ObserveSeconds * 1000)
            $lastText = ''
            try {
                for ($i = 0; $i -lt 2048; $i++) {
                    $event = Read-DesktopProbeFrame $pipe $deadline.Token
                    if (Invoke-DesktopProbeControl $pipe $event $deadline.Token) { continue }
                    if ($event.type -cne 'broadcast' -or
                        ($null -ne $event.targetClientIds -and $clientId -cnotin $event.targetClientIds)) { continue }
                    if ($event.method -ceq 'client-status-changed' -and $event.params.clientId -ceq $followingOwner -and
                        $event.params.status -ceq 'disconnected') { throw 'owner_disconnected' }
                    if ($event.method -ceq 'ipc-connection-reset') { throw 'owner_disconnected' }
                    if ($event.method -cne 'thread-stream-state-changed' -or $event.sourceClientId -cne $followingOwner -or
                        $event.params.hostId -cne 'local' -or $event.params.conversationId -cne $CodexTestId) { continue }
                    if ($event.version -cne 13) { throw 'unsupported_stream_version' }
                    $change = $event.params.change
                    if ($change.type -ceq 'patches') {
                        $stats.omitted_patch_values += @($change.patches | Where-Object { $null -ne $_ -and $_.op -cne 'remove' -and -not $_.Contains('value') }).Count
                    }
                    if (-not (Update-DesktopProbeStream $current $event.params.change $CodexTestId)) {
                        $stats.stale_events++; continue
                    }
                    if ($change.type -ceq 'snapshot') { $stats.snapshot_events++ }
                    else { $stats.patch_events++ }
                    if ($StopClientTestTurn) {
                        $targetTurn = Get-DesktopProbeStopTarget $current.conversationState $CodexTestId $ClientTestMarker
                        if (-not $targetTurn) { continue }
                        $stage = 'codex_test_interrupt'
                        $report.codex_test_interrupt = [ordered]@{ send_attempted = $true; outcome = 'unconfirmed';
                            target_turn_id = $targetTurn; expected_turn_id_used = $true; interrupted_snapshot = $false }
                        $report | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $outputPath 'result.json')
                        $reply = Invoke-DesktopProbeRequest $pipe thread-follower-interrupt-turn @{
                            conversationId = $CodexTestId; mode = 'user-stop'; expectedTurnId = $targetTurn
                        } $clientId -Target $followingOwner -AllowTestInterrupt -TimeoutMs 15000
                        if ($reply.resultType -cne 'success' -or $reply.result.ok -cne $true -or
                            $reply.result.interruptedTurnId -cne $targetTurn -or $reply.result.goalPauseError) { throw 'test_interrupt_unconfirmed' }
                        $report.codex_test_interrupt.outcome = 'acknowledged'
                        Send-DesktopProbeFollowing $pipe $clientId $followingOwner $CodexTestId $true
                        $final = Receive-DesktopProbeSnapshot $pipe $clientId $followingOwner $CodexTestId -ExpectedVersion 13
                        if ($final.conversationState.id -cne $CodexTestId -or $final.conversationState.hostId -cne 'local') {
                            throw 'snapshot_identity_mismatch'
                        }
                        $selected = @(Get-DesktopProbeTurns $final.conversationState | Where-Object { $_.turnId -ceq $targetTurn })
                        $report.codex_test_interrupt.interrupted_snapshot = $selected.Count -eq 1 -and $selected[0].status -ceq 'interrupted'
                        if (-not $report.codex_test_interrupt.interrupted_snapshot) { throw 'test_interrupt_unverified' }
                        break
                    }
                    $turns = @(Get-DesktopProbeTurns $current.conversationState | Where-Object {
                        @($_.params.input | Where-Object { $_.type -ceq 'text' -and $_.text.Contains($ClientTestMarker) }).Count -gt 0
                    })
                    $text = @($turns.items | Where-Object { $_.type -ceq 'agentMessage' } | ForEach-Object { $_.text }) -join ''
                    if ($text -and $text -cne $lastText) {
                        if ($change.type -ceq 'patches') {
                            $stats.text_updates++
                            if ($text -cne $ClientTestMarker) { $stats.partial_text_updates++ }
                        }
                        $lastText = $text
                    }
                    $verification = Test-DesktopProbeClientReply $current.conversationState $CodexTestId $ClientTestMarker
                    if ($change.type -ceq 'patches' -and $verification.reply_matches -and $verification.completed -and $verification.no_tool_items) {
                        $stats.completed_from_patches = $true; break
                    }
                }
            } finally { $deadline.Dispose() }
            if (-not $StopClientTestTurn) {
                if (-not $stats.completed_from_patches) { throw 'client_stream_unverified' }
                Send-DesktopProbeFollowing $pipe $clientId $followingOwner $CodexTestId $true
                $final = Receive-DesktopProbeSnapshot $pipe $clientId $followingOwner $CodexTestId -ExpectedVersion 13
                $verification = Test-DesktopProbeClientReply $final.conversationState $CodexTestId $ClientTestMarker
                $stats.final_snapshot_matches = $verification.reply_matches -and $verification.completed -and $verification.no_tool_items
                $report.codex_client_reverse = $verification
                if (-not $stats.final_snapshot_matches -or $stats.text_updates -lt 1) { throw 'client_stream_unverified' }
            } elseif (-not $report.codex_test_interrupt.interrupted_snapshot) { throw 'test_interrupt_unverified' }
        }
        if ($CodexTestTurn) {
            # First IPC test write only; never replay into the same test thread.
            if ($SnapshotVersion -ne 13) { throw 'test_write_requires_v13' }
            Assert-DesktopProbeTestWriteReady $state
            $stage = 'codex_test_turn'
            $marker = 'DESKTOP_PET_M6_IPC_' + [guid]::NewGuid().ToString('N')
            $messageId = [guid]::NewGuid().ToString()
            $report.codex_test_turn = [ordered]@{ send_attempted = $true; outcome = 'unconfirmed';
                expected_test_marker = $marker; client_message_id = $messageId }
            # Preserve correlation before any possible write, including process interruption.
            $report | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $outputPath 'result.json')
            $pending = [Collections.Generic.Queue[object]]::new()
            $buffer = {
                param($event)
                if ($pending.Count -ge 64) { throw 'event_queue_overflow' }
                $pending.Enqueue($event)
            }
            $parameters = New-DesktopProbeTestTurn $CodexTestId $marker $messageId
            $reply = Invoke-DesktopProbeRequest $pipe thread-follower-start-turn $parameters `
                $clientId -Target $followingOwner -AllowTestTurn -OnBroadcast $buffer -TimeoutMs 30000
            if ($reply.resultType -cne 'success') { throw 'test_turn_rejected_or_unconfirmed' }
            $turnId = $reply.result.result.turn.id
            if ($turnId -isnot [string] -or -not $turnId) { throw 'test_turn_ack_unrecognized' }
            $report.codex_test_turn.outcome = 'acknowledged'
            $report.codex_test_turn.patch_events = 0
            # Inspect the selected stream for a bounded window, then request current state.
            $deadline = [Threading.CancellationTokenSource]::new(15000)
            try {
                for ($i = 0; $i -lt 512; $i++) {
                    $event = if ($pending.Count) { $pending.Dequeue() }
                        else { Read-DesktopProbeFrame $pipe $deadline.Token }
                    if (Invoke-DesktopProbeControl $pipe $event $deadline.Token) { continue }
                    if ($event.type -cne 'broadcast' -or $event.method -cne 'thread-stream-state-changed' -or
                        $event.sourceClientId -cne $followingOwner -or $event.params.hostId -cne 'local' -or
                        $event.params.conversationId -cne $CodexTestId -or
                        ($null -ne $event.targetClientIds -and $clientId -cnotin $event.targetClientIds)) { continue }
                    if ($event.version -cne 13) { throw 'unsupported_stream_version' }
                    if ($event.params.change.type -ceq 'patches') { $report.codex_test_turn.patch_events++ }
                }
            } catch {
                if ($_.Exception.GetBaseException() -isnot [OperationCanceledException]) { throw }
            } finally { $deadline.Dispose() }
            Send-DesktopProbeFollowing $pipe $clientId $followingOwner $CodexTestId $true
            $snapshot = Receive-DesktopProbeSnapshot $pipe $clientId $followingOwner $CodexTestId -ExpectedVersion 13
            $state = $snapshot.conversationState
            if ($state.id -cne $CodexTestId -or $state.hostId -cne 'local') { throw 'snapshot_identity_mismatch' }
            $report.codex_test_verification = Test-DesktopProbeReply $state $CodexTestId $marker $messageId
            $matchedTurn = @(Get-DesktopProbeTurns $state | Where-Object { $_.params.clientUserMessageId -ceq $messageId })
            $report.codex_test_verification.ack_turn_matches = $matchedTurn.Count -eq 1 -and $matchedTurn[0].turnId -ceq $turnId
            if ($false -cin $report.codex_test_verification.Values -or $report.codex_test_turn.patch_events -lt 1) {
                throw 'test_reply_unverified'
            }
        }
    }
    $report.status = 'ok'
} catch {
    $report.failed_stage = $stage
    $report.failure_type = $_.Exception.GetType().Name
    $report.failure_id = $_.FullyQualifiedErrorId
    $report.failure_line = $_.InvocationInfo.ScriptLineNumber
    $report.failure_stack = $_.ScriptStackTrace -replace [regex]::Escape($PSScriptRoot), '<probe>'
    $reason = $_.Exception.GetBaseException().Message
    if ($reason -cin @('unsupported_stream_version', 'invalid_stream_change', 'unsupported_stream_change',
        'owner_disconnected', 'unmatched_frame_limit', 'invalid_frame_size', 'invalid_frame',
        'invalid_response', 'codex_owner_required', 'unexpected_discovery', 'snapshot_identity_mismatch',
        'test_thread_not_ready', 'test_write_already_attempted', 'test_write_requires_v13',
        'event_queue_overflow', 'test_turn_rejected_or_unconfirmed',
        'test_turn_ack_unrecognized', 'response_owner_mismatch', 'test_turn_not_found',
        'test_reply_unverified', 'unsupported_history_shape', 'client_reply_unverified',
        'invalid_state_patch', 'stream_revision_gap', 'client_test_already_present', 'client_stream_unverified',
        'stop_test_has_tools', 'test_interrupt_unconfirmed', 'test_interrupt_unverified')) {
        $report.failure_reason = $reason
    }
    if ($_.Exception.Data.Contains('stream_version')) {
        $report.observed_stream_version = $_.Exception.Data['stream_version']
    }
    if ($_.Exception.Data.Contains('patch_op')) {
        $report.failed_patch = @{ op = $_.Exception.Data['patch_op']; path_types = $_.Exception.Data['patch_path_types'] }
    }
} finally {
    if ($pipe -and $followingOwner) {
        try { Send-DesktopProbeFollowing $pipe $clientId $followingOwner $CodexTestId $false } catch { }
    }
    if ($pipe) { $pipe.Dispose() }
    $report | ConvertTo-Json -Depth 4 | Tee-Object -FilePath (Join-Path $outputPath 'result.json')
}
if ($report.status -ne 'ok') { exit 1 }
