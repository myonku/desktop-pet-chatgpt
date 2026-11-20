#Requires -Version 7.4
# Offline fault checks. No connection to the real desktop client is made.
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'desktop_ipc.psm1') -Force
Add-Type -TypeDefinition @'
using System;
using System.IO;
using System.Threading;
using System.Threading.Tasks;
public class ProbeTranscriptStream : Stream {
    readonly MemoryStream incoming;
    public readonly MemoryStream Written = new MemoryStream();
    public bool Stall;
    public ProbeTranscriptStream(byte[] bytes) { incoming = new MemoryStream(bytes); }
    public override bool CanRead => true;
    public override bool CanWrite => true;
    public override bool CanSeek => false;
    public override long Length => throw new NotSupportedException();
    public override long Position { get => throw new NotSupportedException(); set => throw new NotSupportedException(); }
    public override int Read(byte[] bytes, int offset, int count) => incoming.Read(bytes, offset, Math.Min(2, count));
    public override async Task<int> ReadAsync(byte[] bytes, int offset, int count, CancellationToken token) {
        if (Stall) await Task.Delay(Timeout.Infinite, token);
        token.ThrowIfCancellationRequested();
        return Read(bytes, offset, count);
    }
    public override void Write(byte[] bytes, int offset, int count) => Written.Write(bytes, offset, count);
    public override void Flush() { }
    public override long Seek(long offset, SeekOrigin origin) => throw new NotSupportedException();
    public override void SetLength(long length) => throw new NotSupportedException();
    protected override void Dispose(bool disposing) { incoming.Dispose(); Written.Dispose(); base.Dispose(disposing); }
}
'@

$requestId = [guid]::NewGuid()
$passed = 0
function Frame($Message) {
    $bytes = [Text.Encoding]::UTF8.GetBytes(($Message | ConvertTo-Json -Compress -Depth 32))
    return ,([byte[]]([BitConverter]::GetBytes([uint32]$bytes.Length) + $bytes))
}
function Reply($Method = 'initialize', $Result = @{ clientId = 'probe-client' }) {
    return @{ type = 'response'; requestId = $requestId.ToString(); method = $Method
        resultType = 'success'; result = $Result }
}
function Check($Name, [scriptblock] $Body) {
    & $Body
    $script:passed++
    Write-Output "PASS $Name"
}
function Reject([byte[]] $Bytes, $Reason) {
    $stream = [ProbeTranscriptStream]::new($Bytes)
    try {
        try {
            $null = Invoke-DesktopProbeRequest $stream initialize @{} initializing-client -RequestId $requestId
        } catch {
            if ($_.Exception.Message.Contains($Reason)) { return }
            throw
        }
        throw "Expected rejection: $Reason"
    } finally { $stream.Dispose() }
}

Check 'fragmented frames, unrelated replies and inbound control requests' {
    $unrelated = Reply
    $unrelated.requestId = [guid]::NewGuid().ToString()
    $bytes = [byte[]]((Frame @{ type = 'client-discovery-request'; requestId = 'discovery' }) +
        (Frame @{ type = 'request'; requestId = 'unsupported'; method = 'unknown' }) +
        (Frame $unrelated) + (Frame (Reply)))
    $stream = [ProbeTranscriptStream]::new($bytes)
    try {
        $reply = Invoke-DesktopProbeRequest $stream initialize @{} initializing-client -RequestId $requestId
        if ($reply.result.clientId -cne 'probe-client') { throw 'wrong_reply' }
        $written = [IO.MemoryStream]::new($stream.Written.ToArray())
        try {
            $messages = @()
            while ($written.Position -lt $written.Length) {
                $prefix = [byte[]]::new(4)
                $written.ReadExactly($prefix)
                $body = [byte[]]::new([BitConverter]::ToUInt32($prefix))
                $written.ReadExactly($body)
                $messages += [Text.Encoding]::UTF8.GetString($body) | ConvertFrom-Json
            }
            if ($messages.Count -ne 3 -or $messages[0].version -ne 0 -or
                $messages[1].type -cne 'client-discovery-response' -or
                $messages[1].requestId -cne 'discovery' -or $messages[1].response.canHandle -cne $false -or
                $messages[2].error -cne 'no-handler-for-request') { throw 'invalid_control_reply' }
        } finally { $written.Dispose() }
    } finally { $stream.Dispose() }
}
Check 'owner discovery request uses version one and preserves rejection' {
    $stream = [ProbeTranscriptStream]::new((Frame @{ type = 'response'; requestId = $requestId.ToString()
        resultType = 'error'; error = 'no-client-found' }))
    try {
        $reply = Invoke-DesktopProbeRequest $stream thread-owner-discovery @{ hostId = 'local'; conversationId = 'test-id' } probe-client -RequestId $requestId
        if ($reply.error -cne 'no-client-found') { throw 'rejection_lost' }
        $written = $stream.Written.ToArray()
        $sent = [Text.Encoding]::UTF8.GetString($written, 4, $written.Length - 4) | ConvertFrom-Json
        if ($sent.version -ne 1 -or $sent.params.conversationId -cne 'test-id') { throw 'wrong_discovery_request' }
    } finally { $stream.Dispose() }
}
Check 'wrong response method' { Reject (Frame (Reply 'other')) 'invalid_response' }
Check 'missing success result' { Reject (Frame (Reply 'initialize' $null)) 'invalid_response' }
Check 'unknown result type' {
    $reply = Reply
    $reply.resultType = 'other'
    Reject (Frame $reply) 'invalid_response'
}
Check 'non-object frame' { Reject (Frame @(1, 2)) 'invalid_frame' }
Check 'zero frame size' { Reject ([byte[]]@(0, 0, 0, 0)) 'invalid_frame_size' }
Check 'oversized frame' { Reject ([BitConverter]::GetBytes([uint32](1MB + 1))) 'invalid_frame_size' }
Check 'invalid UTF8' { Reject ([byte[]]@(1, 0, 0, 0, 255)) 'Unable to translate' }
Check 'truncated frame' { Reject ([byte[]]@(2, 0, 0, 0, 123)) 'end of the stream' }
Check 'bounded unmatched frames' {
    $bytes = [byte[]]::new(0)
    1..64 | ForEach-Object { $bytes += Frame @{ type = 'broadcast'; method = 'unknown' } }
    Reject $bytes 'unmatched_frame_limit'
}
Check 'nonresponsive stream cancels' {
    $stream = [ProbeTranscriptStream]::new([byte[]]::new(0))
    $stream.Stall = $true
    $timer = [Diagnostics.Stopwatch]::StartNew()
    try {
        $cancelled = $false
        try { $null = Invoke-DesktopProbeRequest $stream initialize @{} probe-client -TimeoutMs 100 }
        catch { $cancelled = $_.Exception.GetBaseException() -is [OperationCanceledException] }
        if (-not $cancelled -or $timer.Elapsed.TotalSeconds -gt 3) { throw 'cancellation_failed' }
    } finally { $stream.Dispose() }
}
Check 'turn writes require explicit opt-in' {
    $stream = [ProbeTranscriptStream]::new([byte[]]::new(0))
    try {
        $rejected = $false
        try { $null = Invoke-DesktopProbeRequest $stream thread-follower-start-turn @{} probe-client }
        catch { $rejected = $_.Exception.Message -ceq 'turn_write_not_enabled' }
        if (-not $rejected -or $stream.Written.Length -ne 0) { throw 'mutation_allowed' }
    } finally { $stream.Dispose() }
}
Check 'targeted test-turn acknowledgement must come from selected owner' {
    $reply = Reply 'thread-follower-start-turn' @{ result = @{ turn = @{ id = 'turn' } } }
    $reply.handledByClientId = 'other-owner'
    $stream = [ProbeTranscriptStream]::new((Frame $reply))
    try {
        $rejected = $false
        try {
            $null = Invoke-DesktopProbeRequest $stream thread-follower-start-turn @{} probe-client `
                -RequestId $requestId -Target owner -AllowTestTurn
        } catch { $rejected = $_.Exception.Message -ceq 'response_owner_mismatch' }
        if (-not $rejected) { throw 'foreign_owner_accepted' }
    } finally { $stream.Dispose() }
}
function SnapshotEvent($Owner = 'owner', $Conversation = 'test-thread', $Version = 13) {
    return @{ type = 'broadcast'; method = 'thread-stream-state-changed'; version = $Version
        sourceClientId = $Owner; targetClientIds = @('probe-client')
        params = @{ hostId = 'local'; conversationId = $Conversation; change = @{
            type = 'snapshot'; revision = 1; conversationState = @{ id = $Conversation; hostId = 'local' }
        } }
    }
}
Check 'snapshot filters owner, conversation and target recipient' {
    $foreignTarget = SnapshotEvent
    $foreignTarget.targetClientIds = @('other-client')
    $stream = [ProbeTranscriptStream]::new([byte[]]((Frame (SnapshotEvent other-owner)) +
        (Frame (SnapshotEvent owner other-thread)) + (Frame $foreignTarget) + (Frame (SnapshotEvent))))
    try {
        $snapshot = Receive-DesktopProbeSnapshot $stream probe-client owner test-thread -ExpectedVersion 13
        if ($snapshot.conversationState.id -cne 'test-thread') { throw 'foreign_snapshot_accepted' }
    } finally { $stream.Dispose() }
}
Check 'stream version drift fails closed' {
    $stream = [ProbeTranscriptStream]::new((Frame (SnapshotEvent)))
    try {
        $rejected = $false
        try { $null = Receive-DesktopProbeSnapshot $stream probe-client owner test-thread }
        catch { $rejected = $_.Exception.Message -ceq 'unsupported_stream_version' -and $_.Exception.Data['stream_version'] -eq 13 }
        if (-not $rejected) { throw 'version_drift_accepted' }
    } finally { $stream.Dispose() }
}
Check 'owner disconnect invalidates the stream' {
    $stream = [ProbeTranscriptStream]::new((Frame @{ type = 'broadcast'; method = 'client-status-changed'
        params = @{ clientId = 'owner'; status = 'disconnected' } }))
    try {
        $rejected = $false
        try { $null = Receive-DesktopProbeSnapshot $stream probe-client owner test-thread }
        catch { $rejected = $_.Exception.Message -ceq 'owner_disconnected' }
        if (-not $rejected) { throw 'disconnected_owner_accepted' }
    } finally { $stream.Dispose() }
}
function TestState {
    $turn = @{ turnId = 'turn'; status = 'completed'; params = @{
        threadId = 'test-thread'; clientUserMessageId = 'test-message' }
        items = @(@{ type = 'userMessage'; content = @(@{ type = 'text'; text = 'Reply with exactly MARKER. Do not use any tools.' }) },
            @{ type = 'agentMessage'; text = 'MARKER' }) }
    return @{ id = 'test-thread'; hostId = 'local'; turns = @(); turnHistory = @{
        history = @{ entitiesByKey = @{ entity = $turn } } } }
}
Check 'paginated history takes precedence over empty legacy turns' {
    $state = TestState
    if (@(Get-DesktopProbeTurns $state).Count -ne 1) { throw 'paginated_history_lost' }
    $result = Test-DesktopProbeReply $state test-thread MARKER test-message
    if ($false -cin $result.Values) { throw 'valid_reply_rejected' }
}
Check 'unknown paginated history cannot masquerade as an empty thread' {
    $state = TestState
    $state.turnHistory = @{ unknown = @{} }
    $rejected = $false
    try { $null = Get-DesktopProbeTurns $state } catch { $rejected = $_.Exception.Message -ceq 'unsupported_history_shape' }
    if (-not $rejected) { throw 'unknown_history_accepted' }
}
Check 'reply verification rejects another message and conversation' {
    $state = TestState
    $rejected = $false
    try { $null = Test-DesktopProbeReply $state test-thread MARKER other-message }
    catch { $rejected = $_.Exception.Message -ceq 'test_turn_not_found' }
    if (-not $rejected) { throw 'foreign_message_accepted' }
    $rejected = $false
    try { $null = Test-DesktopProbeReply $state other-thread MARKER test-message }
    catch { $rejected = $_.Exception.Message -ceq 'snapshot_identity_mismatch' }
    if (-not $rejected) { throw 'foreign_conversation_accepted' }
}
Check 'wrong text, incomplete turn and tool use cannot verify a reply' {
    $state = TestState
    $turn = $state.turnHistory.history.entitiesByKey.entity
    $turn.items[1].text = 'wrong'
    $turn.status = 'inProgress'
    $turn.items += @{ type = 'commandExecution' }
    $result = Test-DesktopProbeReply $state test-thread MARKER test-message
    if ($result.reply_matches -or $result.completed -or $result.no_tool_items) { throw 'invalid_reply_accepted' }
}
Check 'first test write allows existing ordinary test history only when idle and complete' {
    $state = TestState
    $state.threadRuntimeStatus = @{ type = 'idle' }
    $state.turnHistory.history.isComplete = $true
    Assert-DesktopProbeTestWriteReady $state
}
Check 'partial history or active state cannot authorize a test write' {
    foreach ($active in @($false, $true)) {
        $state = TestState
        $state.threadRuntimeStatus = @{ type = $(if ($active) { 'active' } else { 'idle' }) }
        $state.turnHistory.history.isComplete = $active
        $rejected = $false
        try { Assert-DesktopProbeTestWriteReady $state }
        catch { $rejected = $_.Exception.Message -ceq 'test_thread_not_ready' }
        if (-not $rejected) { throw 'unsafe_test_state_accepted' }
    }
}
Check 'prior test write prevents replay regardless of its completion status' {
    $state = TestState
    $state.threadRuntimeStatus = @{ type = 'idle' }
    $state.turnHistory.history.isComplete = $true
    $turn = $state.turnHistory.history.entitiesByKey.entity
    $turn.status = 'failed'
    $turn.params.input = @(@{ type = 'text'; text = 'Reply DESKTOP_PET_M6_IPC_0123456789abcdef0123456789abcdef' })
    $rejected = $false
    try { Assert-DesktopProbeTestWriteReady $state }
    catch { $rejected = $_.Exception.Message -ceq 'test_write_already_attempted' }
    if (-not $rejected) { throw 'test_write_replayed' }
}
Check 'client-originated reply is verified in selected history' {
    $state = TestState
    $turn = $state.turnHistory.history.entitiesByKey.entity
    $turn.items[0].content[0].text = '只回复 DESKTOP_PET_M6_CLIENT_OK，不调用工具'
    $turn.items[1].text = 'DESKTOP_PET_M6_CLIENT_OK'
    $result = Test-DesktopProbeClientReply $state test-thread
    if (-not $result.reply_matches -or -not $result.completed -or -not $result.no_tool_items) { throw 'client_reply_rejected' }
}
Check 'agent marker alone cannot verify a client-originated round trip' {
    $state = TestState
    $state.turnHistory.history.entitiesByKey.entity.items[1].text = 'DESKTOP_PET_M6_CLIENT_OK'
    $result = Test-DesktopProbeClientReply $state test-thread
    if ($result.reply_matches -or $result.matching_turns -ne 0) { throw 'unrelated_marker_accepted' }
}
Check 'plain text test input satisfies desktop pre-normalization contract on the wire' {
    $stream = [ProbeTranscriptStream]::new((Frame @{ type = 'response'; requestId = $requestId.ToString()
        method = 'thread-follower-start-turn'; resultType = 'success'; handledByClientId = 'owner'; result = @{} }))
    try {
        $parameters = New-DesktopProbeTestTurn test-thread MARKER test-message
        $null = Invoke-DesktopProbeRequest $stream thread-follower-start-turn $parameters probe-client `
            -RequestId $requestId -Target owner -AllowTestTurn
        $bytes = $stream.Written.ToArray()
        $sent = [Text.Encoding]::UTF8.GetString($bytes, 4, $bytes.Length - 4) | ConvertFrom-Json -AsHashtable
        $input = $sent.params.turnStart.request.input
        if ($input.Count -ne 1 -or -not $input[0].Contains('text_elements') -or
            $input[0].text_elements -isnot [array] -or $input[0].text_elements.Count -ne 0 -or
            $input[0].text -cne 'Reply with exactly MARKER. Do not use any tools.' -or
            $sent.params.turnStart.request.threadId -cne 'test-thread' -or
            $sent.params.turnStart.request.clientUserMessageId -cne 'test-message') { throw 'desktop_input_contract_invalid' }
    } finally { $stream.Dispose() }
}
Check 'stream patches reconstruct nested text, array insertions, replacements and removals' {
    $current = @{ revision = 4; conversationState = (TestState) }
    $change = @{ type = 'patches'; baseRevision = 4; revision = 5; patches = @(
        @{ op = 'replace'; path = @('turnHistory','history','entitiesByKey','entity','items',1,'text'); value = 'NEW' },
        @{ op = 'add'; path = @('turnHistory','history','entitiesByKey','entity','items',2); value = @{ type = 'reasoning' } },
        @{ op = 'replace'; path = @('turnHistory','history','entitiesByKey','entity','items',0); value = @{ type = 'userMessage' } },
        @{ op = 'remove'; path = @('turnHistory','history','entitiesByKey','entity','items',2) },
        @{ op = 'add'; path = @('extra'); value = 'temporary' },
        @{ op = 'remove'; path = @('extra') }
    ) }
    if (-not (Update-DesktopProbeStream $current $change test-thread) -or $current.revision -ne 5) { throw 'revision_not_advanced' }
    $items = $current.conversationState.turnHistory.history.entitiesByKey.entity.items
    if ($items.Count -ne 2 -or $items[1].text -cne 'NEW' -or $current.conversationState.Contains('extra')) { throw 'patch_reconstruction_failed' }
}
Check 'duplicate and out of order patches do not alter accepted state' {
    $current = @{ revision = 5; conversationState = (TestState) }
    $change = @{ type = 'patches'; baseRevision = 4; revision = 5; patches = @(
        @{ op = 'replace'; path = @('id'); value = 'other' }) }
    if ((Update-DesktopProbeStream $current $change test-thread) -or $current.conversationState.id -cne 'test-thread') { throw 'duplicate_applied' }
}
Check 'missing stream revisions require a new snapshot' {
    $current = @{ revision = 5; conversationState = (TestState) }
    $rejected = $false
    try { $null = Update-DesktopProbeStream $current @{ type = 'patches'; baseRevision = 6; revision = 7; patches = @() } test-thread }
    catch { $rejected = $_.Exception.Message -ceq 'stream_revision_gap' }
    if (-not $rejected -or $current.revision -ne 5) { throw 'gap_accepted' }
}
Check 'invalid patch batch cannot partially change accepted state' {
    $current = @{ revision = 5; conversationState = (TestState) }
    $rejected = $false
    try { $null = Update-DesktopProbeStream $current @{ type = 'patches'; baseRevision = 5; revision = 6; patches = @(
        @{ op = 'add'; path = @('extra'); value = 'temporary' },
        @{ op = 'remove'; path = @('missing','child') }) } test-thread }
    catch { $rejected = $_.Exception.Message -ceq 'invalid_state_patch' }
    if (-not $rejected -or $current.revision -ne 5 -or $current.conversationState.Contains('extra')) { throw 'partial_batch_accepted' }
}
Check 'patches cannot change bound thread identity' {
    $current = @{ revision = 5; conversationState = (TestState) }
    $rejected = $false
    try { $null = Update-DesktopProbeStream $current @{ type = 'patches'; baseRevision = 5; revision = 6; patches = @(
        @{ op = 'replace'; path = @('hostId'); value = 'remote' }) } test-thread }
    catch { $rejected = $_.Exception.Message -ceq 'snapshot_identity_mismatch' }
    if (-not $rejected -or $current.conversationState.hostId -cne 'local') { throw 'identity_change_accepted' }
}
Check 'invalid array indexes and unknown patch operations are rejected' {
    foreach ($patch in @(
        @{ op = 'replace'; path = @('turns',0); value = @{} },
        @{ op = 'add'; path = @('turns',-1); value = @{} },
        @{ op = 'add'; path = @('turns','0'); value = @{} },
        @{ op = 'copy'; path = @('id'); value = 'other' })) {
        $current = @{ revision = 5; conversationState = (TestState) }
        $rejected = $false
        try { $null = Update-DesktopProbeStream $current @{ type = 'patches'; baseRevision = 5; revision = 6; patches = @($patch) } test-thread }
        catch { $rejected = $_.Exception.Message -ceq 'invalid_state_patch' }
        if (-not $rejected) { throw 'invalid_patch_accepted' }
    }
}
Check 'undefined Immer values omitted by JSON are cleared without losing array slots' {
    $current = @{ revision = 5; conversationState = (TestState) }
    $change = @{ type = 'patches'; baseRevision = 5; revision = 6; patches = @(
        @{ op = 'replace'; path = @('turnHistory','history','entitiesByKey','entity','error') },
        @{ op = 'replace'; path = @('turnHistory','history','entitiesByKey','entity','items',1) }
    ) }
    $current.conversationState.turnHistory.history.entitiesByKey.entity.error = 'old'
    $null = Update-DesktopProbeStream $current $change test-thread
    $turn = $current.conversationState.turnHistory.history.entitiesByKey.entity
    if ($turn.Contains('error') -or $turn.items.Count -ne 2 -or $null -ne $turn.items[1]) { throw 'undefined_patch_incorrect' }
}
Check 'stream snapshot rebases revision before subsequent text patches' {
    $current = @{ revision = 1; conversationState = (TestState) }
    $snapshot = @{ type = 'snapshot'; revision = 8; conversationState = (TestState) }
    if (-not (Update-DesktopProbeStream $current $snapshot test-thread)) { throw 'snapshot_not_applied' }
    $patch = @{ type = 'patches'; baseRevision = 8; revision = 9; patches = @(
        @{ op = 'replace'; path = @('turnHistory','history','entitiesByKey','entity','items',1,'text'); value = 'NEW' }) }
    $null = Update-DesktopProbeStream $current $patch test-thread
    if ($current.revision -ne 9 -or $current.conversationState.turnHistory.history.entitiesByKey.entity.items[1].text -cne 'NEW') { throw 'snapshot_rebase_failed' }
}
Check 'snapshot wait responds to other clients discovery requests' {
    $snapshot = @{ type = 'broadcast'; method = 'thread-stream-state-changed'; version = 13
        sourceClientId = 'owner'; params = @{ hostId = 'local'; conversationId = 'test-thread'
            change = @{ type = 'snapshot'; revision = 1; conversationState = (TestState) } } }
    $stream = [ProbeTranscriptStream]::new([byte[]]((Frame @{ type = 'client-discovery-request'; requestId = 'other-discovery' }) + (Frame $snapshot)))
    try {
        $null = Receive-DesktopProbeSnapshot $stream probe-client owner test-thread -ExpectedVersion 13
        $bytes = $stream.Written.ToArray()
        $reply = [Text.Encoding]::UTF8.GetString($bytes, 4, $bytes.Length - 4) | ConvertFrom-Json
        if ($reply.requestId -cne 'other-discovery' -or $reply.response.canHandle -cne $false) { throw 'discovery_not_answered' }
    } finally { $stream.Dispose() }
}
Check 'interrupt needs explicit opt-in, selected owner and expected turn id' {
    foreach ($parameters in @(@{ mode = 'user-stop' }, @{ mode = 'system'; expectedTurnId = 'turn' })) {
        $stream = [ProbeTranscriptStream]::new([byte[]]::new(0))
        try {
            $rejected = $false
            try { $null = Invoke-DesktopProbeRequest $stream thread-follower-interrupt-turn $parameters probe-client -Target owner -AllowTestInterrupt }
            catch { $rejected = $_.Exception.Message -ceq 'turn_interrupt_not_enabled' }
            if (-not $rejected -or $stream.Written.Length) { throw 'unscoped_interrupt_sent' }
        } finally { $stream.Dispose() }
    }
    $stream = [ProbeTranscriptStream]::new([byte[]]::new(0))
    try {
        $rejected = $false
        try { $null = Invoke-DesktopProbeRequest $stream thread-follower-interrupt-turn @{ mode = 'user-stop'; expectedTurnId = 'turn' } probe-client -Target owner }
        catch { $rejected = $_.Exception.Message -ceq 'turn_interrupt_not_enabled' }
        if (-not $rejected -or $stream.Written.Length) { throw 'interrupt_opt_in_bypassed' }
    } finally { $stream.Dispose() }
}
Check 'interrupt request uses v4 with the observed active turn id' {
    $reply = Reply 'thread-follower-interrupt-turn' @{ ok = $true; interruptedTurnId = 'turn' }
    $reply.handledByClientId = 'owner'
    $stream = [ProbeTranscriptStream]::new((Frame $reply))
    try {
        $null = Invoke-DesktopProbeRequest $stream thread-follower-interrupt-turn @{
            conversationId = 'test-thread'; mode = 'user-stop'; expectedTurnId = 'turn'
        } probe-client -RequestId $requestId -Target owner -AllowTestInterrupt
        $bytes = $stream.Written.ToArray()
        $sent = [Text.Encoding]::UTF8.GetString($bytes,4,$bytes.Length-4) | ConvertFrom-Json
        if ($sent.version -ne 4 -or $sent.params.expectedTurnId -cne 'turn' -or $sent.targetClientId -cne 'owner') { throw 'interrupt_scope_lost' }
    } finally { $stream.Dispose() }
}
Check 'only a matching active tool-free client test is eligible for interruption' {
    $state = TestState
    $turn = $state.turnHistory.history.entitiesByKey.entity
    $turn.params.input = @(@{ type = 'text'; text = 'DESKTOP_PET_M6_CLIENT_STOP_01 count to 10000' })
    $turn.status = 'inProgress'
    $state.threadRuntimeStatus = @{ type = 'active' }
    if ((Get-DesktopProbeStopTarget $state test-thread DESKTOP_PET_M6_CLIENT_STOP_01) -cne 'turn') { throw 'stop_target_not_found' }
    if ($null -ne (Get-DesktopProbeStopTarget $state test-thread DESKTOP_PET_M6_CLIENT_STOP_02)) { throw 'other_message_selected' }
    $turn.status = 'completed'
    if ($null -ne (Get-DesktopProbeStopTarget $state test-thread DESKTOP_PET_M6_CLIENT_STOP_01)) { throw 'completed_turn_selected' }
    $turn.status = 'inProgress'
    $turn.items += @{ type = 'commandExecution' }
    $rejected = $false
    try { $null = Get-DesktopProbeStopTarget $state test-thread DESKTOP_PET_M6_CLIENT_STOP_01 }
    catch { $rejected = $_.Exception.Message -ceq 'stop_test_has_tools' }
    if (-not $rejected) { throw 'tool_turn_selected' }
}
Check 'unconfirmed-send fault writes exactly one valid frame without reading any reply' {
    $stream = [ProbeTranscriptStream]::new([byte[]]::new(0))
    $stream.Stall = $true
    try {
        $marker = 'DESKTOP_PET_M6_IPC_0123456789abcdef0123456789abcdef'
        $message = '01234567-89ab-cdef-0123-456789abcdef'
        Send-DesktopProbeTestFrame $stream test-thread $marker $message probe-client owner -RequestId $requestId -AllowTestTurn
        $bytes = $stream.Written.ToArray()
        if ($bytes.Length -ne 4 + [BitConverter]::ToUInt32($bytes,0)) { throw 'multiple_frames_sent' }
        $sent = [Text.Encoding]::UTF8.GetString($bytes,4,$bytes.Length-4) | ConvertFrom-Json
        if ($sent.version -ne 2 -or $sent.requestId -cne $requestId.ToString() -or
            $sent.params.turnStart.request.clientUserMessageId -cne $message -or
            $sent.params.turnStart.request.input[0].text_elements -isnot [array]) { throw 'unconfirmed_frame_invalid' }
    } finally { $stream.Dispose() }
}
Check 'write-only fault helper still requires explicit write authorization' {
    $stream = [ProbeTranscriptStream]::new([byte[]]::new(0))
    try {
        $rejected = $false
        try { Send-DesktopProbeTestFrame $stream test-thread DESKTOP_PET_M6_IPC_0123456789abcdef0123456789abcdef 01234567-89ab-cdef-0123-456789abcdef probe-client owner }
        catch { $rejected = $_.Exception.Message -ceq 'turn_write_not_enabled' }
        if (-not $rejected -or $stream.Written.Length) { throw 'unconfirmed_send_authorization_bypassed' }
    } finally { $stream.Dispose() }
}
Check 'recovery stream filters routes and answers discovery before returning selected change' {
    $foreign = @{ type = 'broadcast'; method = 'thread-stream-state-changed'; version = 13; sourceClientId = 'other-owner'
        params = @{ hostId = 'local'; conversationId = 'test-thread'; change = @{ type = 'patches' } } }
    $selected = @{ type = 'broadcast'; method = 'thread-stream-state-changed'; version = 13; sourceClientId = 'owner'
        targetClientIds = @('probe-client'); params = @{ hostId = 'local'; conversationId = 'test-thread'
            change = @{ type = 'patches'; baseRevision = 1; revision = 2; patches = @() } } }
    $stream = [ProbeTranscriptStream]::new([byte[]]((Frame $foreign) +
        (Frame @{ type = 'client-discovery-request'; requestId = 'recovery-discovery' }) + (Frame $selected)))
    try {
        $session = @{ pipe = $stream; client = 'probe-client'; owner = 'owner'; conversation = 'test-thread' }
        $change = Receive-DesktopProbeSessionChange $session ([Threading.CancellationToken]::None)
        if ($change.revision -ne 2 -or $stream.Written.Length -eq 0) { throw 'recovery_route_filter_failed' }
    } finally { $stream.Dispose() }
}
Check 'recovery stream cannot reuse a disconnected owner or unknown version' {
    foreach ($event in @(
        @{ type = 'broadcast'; method = 'ipc-connection-reset'; params = @{} },
        @{ type = 'broadcast'; method = 'thread-stream-state-changed'; version = 14; sourceClientId = 'owner'
            params = @{ hostId = 'local'; conversationId = 'test-thread'; change = @{ type = 'patches' } } })) {
        $stream = [ProbeTranscriptStream]::new((Frame $event))
        try {
            $rejected = $false
            try { $null = Receive-DesktopProbeSessionChange @{ pipe = $stream; client = 'probe-client'; owner = 'owner'; conversation = 'test-thread' } ([Threading.CancellationToken]::None) }
            catch { $rejected = $_.Exception.Message -cin @('owner_disconnected','unsupported_stream_version') }
            if (-not $rejected) { throw 'invalid_recovery_stream_accepted' }
        } finally { $stream.Dispose() }
    }
}
Check 'concurrent write acknowledgements retain request and owner correlation' {
    $other = Reply 'thread-follower-start-turn' @{ result = @{ turn = @{ id = 'other-turn' } } }
    $other.requestId = [guid]::NewGuid().ToString()
    $other.handledByClientId = 'owner'
    $selected = Reply 'thread-follower-start-turn' @{ result = @{ turn = @{ id = 'selected-turn' } } }
    $selected.handledByClientId = 'owner'
    $stream = [ProbeTranscriptStream]::new([byte[]]((Frame $other) +
        (Frame @{ type = 'client-discovery-request'; requestId = 'other-client' }) + (Frame $selected)))
    try {
        $reply = Receive-DesktopProbeTestAcknowledgement @{ pipe = $stream; owner = 'owner' } $requestId
        if ($reply.result.result.turn.id -cne 'selected-turn' -or $stream.Written.Length -eq 0) { throw 'concurrent_ack_misrouted' }
    } finally { $stream.Dispose() }
}
Check 'concurrent write acknowledgement cannot come from another owner' {
    $reply = Reply 'thread-follower-start-turn' @{ result = @{ turn = @{ id = 'turn' } } }
    $reply.handledByClientId = 'other-owner'
    $stream = [ProbeTranscriptStream]::new((Frame $reply))
    try {
        $rejected = $false
        try { $null = Receive-DesktopProbeTestAcknowledgement @{ pipe = $stream; owner = 'owner' } $requestId }
        catch { $rejected = $_.Exception.Message -ceq 'response_owner_mismatch' }
        if (-not $rejected) { throw 'foreign_concurrent_ack_accepted' }
    } finally { $stream.Dispose() }
}
Check 'idle runtime with an optimistic in-progress row cannot authorize another write' {
    $state = TestState
    $state.threadRuntimeStatus = @{ type = 'idle' }
    $state.turnHistory.history.isComplete = $true
    $turn = $state.turnHistory.history.entitiesByKey.entity
    $turn.status = 'inProgress'
    $turn.turnId = $null
    $rejected = $false
    try { Assert-DesktopProbeTestWriteReady $state }
    catch { $rejected = $_.Exception.Message -ceq 'test_thread_not_ready' }
    if (-not $rejected) { throw 'optimistic_pending_write_accepted' }
}
Check 'unconfirmed submission blocks another write even with idle completed history' {
    $state = TestState
    $state.threadRuntimeStatus = @{ type = 'idle' }
    $state.turnHistory.history.isComplete = $true
    $state.unconfirmedTurnSubmissions = @(@{ requestId = 'pending'; terminal = $true })
    $rejected = $false
    try { Assert-DesktopProbeTestWriteReady $state }
    catch { $rejected = $_.Exception.Message -ceq 'test_thread_not_ready' }
    if (-not $rejected) { throw 'unconfirmed_pending_write_accepted' }
}
function QueueParameters([switch] $Reserve) {
    return New-DesktopProbeQueueRequest @{ id = 'test-thread'; hostId = 'local'; cwd = 'C:/m6-test' } test-thread `
        DESKTOP_PET_M6_QUEUE_0123456789abcdef0123456789abcdef 01234567-89ab-cdef-0123-456789abcdef -Reserve:$Reserve
}
Check 'queue reservation and completion use one incremental message and inherited context' {
    foreach ($reserve in @($true,$false)) {
        $parameters = QueueParameters -Reserve:$reserve
        Assert-DesktopProbeQueueRequest $parameters
        $message = $parameters.state['test-thread'][0]
        if ($parameters.state.Count -ne 1 -or $parameters.state['test-thread'].Count -ne 1 -or
            $parameters.discardedMessageIds.Count -ne 0 -or $message.Contains('preparation') -ne $reserve -or
            $parameters.preparationMessageId -cne $message.id) { throw 'queue_incremental_contract_lost' }
    }
}
Check 'queue writes require explicit opt-in before any frame is sent' {
    $stream = [ProbeTranscriptStream]::new([byte[]]::new(0))
    try {
        $rejected = $false
        try { $null = Invoke-DesktopProbeRequest $stream thread-follower-set-queued-follow-ups-state (QueueParameters -Reserve) probe-client -Target owner }
        catch { $rejected = $_.Exception.Message -ceq 'test_queue_not_enabled' }
        if (-not $rejected -or $stream.Written.Length) { throw 'queue_opt_in_bypassed' }
    } finally { $stream.Dispose() }
}
Check 'queue mutation cannot replace whole state, discard other messages or carry attachments' {
    foreach ($kind in @('no-reservation-id','extra-thread','extra-message','discard','attachment','permissions','wrong-id')) {
        $parameters = QueueParameters -Reserve
        switch ($kind) {
            'no-reservation-id' { $parameters.Remove('preparationMessageId') }
            'extra-thread' { $parameters.state.other = @() }
            'extra-message' { $parameters.state['test-thread'] += $parameters.state['test-thread'][0] }
            'discard' { $parameters.discardedMessageIds = @('other') }
            'attachment' { $parameters.state['test-thread'][0].context.addedFiles = @('other') }
            'permissions' { $parameters.state['test-thread'][0].context.workPermissions = @{ mode = 'full-access' } }
            'wrong-id' { $parameters.preparationMessageId = '11111111-1111-1111-1111-111111111111' }
        }
        $rejected = $false
        try { Assert-DesktopProbeQueueRequest $parameters }
        catch { $rejected = $_.Exception.Message -ceq 'invalid_test_queue_request' }
        if (-not $rejected) { throw "unsafe_queue_request_accepted:$kind" }
    }
}
Check 'queue request uses protocol v3 and retains reservation ID on the wire' {
    $reply = Reply 'thread-follower-set-queued-follow-ups-state' @{ ok = $true }
    $reply.handledByClientId = 'owner'
    $stream = [ProbeTranscriptStream]::new((Frame $reply))
    try {
        $null = Invoke-DesktopProbeRequest $stream thread-follower-set-queued-follow-ups-state (QueueParameters -Reserve) `
            probe-client -Target owner -AllowTestQueue -RequestId $requestId
        $bytes = $stream.Written.ToArray()
        $sent = [Text.Encoding]::UTF8.GetString($bytes,4,$bytes.Length-4) | ConvertFrom-Json -AsHashtable
        if ($sent.version -ne 3 -or $sent.params.preparationMessageId -cne '01234567-89ab-cdef-0123-456789abcdef' -or
            $sent.params.state['test-thread'][0].preparation -isnot [System.Collections.IDictionary]) { throw 'queue_wire_contract_lost' }
    } finally { $stream.Dispose() }
}
Check 'queue cleanup is separately gated and cannot clear a whole queue' {
    $stream = [ProbeTranscriptStream]::new([byte[]]::new(0))
    try {
        $rejected = $false
        try { $null = Invoke-DesktopProbeRequest $stream thread-follower-remove-queued-message @{
            conversationId = 'test-thread'; messageId = '01234567-89ab-cdef-0123-456789abcdef'
        } probe-client -Target owner }
        catch { $rejected = $_.Exception.Message -ceq 'test_queue_removal_not_enabled' }
        if (-not $rejected -or $stream.Written.Length) { throw 'queue_cleanup_gate_bypassed' }
    } finally { $stream.Dispose() }
}
Check 'existing queue test marker prevents replay through direct-send experiment' {
    $state = TestState
    $state.threadRuntimeStatus = @{ type = 'idle' }
    $state.turnHistory.history.isComplete = $true
    $state.turnHistory.history.entitiesByKey.entity.params.input = @(@{ type = 'text'; text = 'DESKTOP_PET_M6_QUEUE_0123456789abcdef0123456789abcdef' })
    $rejected = $false
    try { Assert-DesktopProbeTestWriteReady $state }
    catch { $rejected = $_.Exception.Message -ceq 'test_write_already_attempted' }
    if (-not $rejected) { throw 'queue_test_replayed' }
}
Check 'only native queue text may normalize one appended linefeed' {
    $state = TestState
    $turn = $state.turnHistory.history.entitiesByKey.entity
    $marker = 'DESKTOP_PET_M6_QUEUE_0123456789abcdef0123456789abcdef'
    $turn.items[0].content[0].text = "Reply with exactly $marker. Do not use any tools.`n"
    $turn.items[1].text = $marker
    if (-not (Test-DesktopProbeReply $state test-thread $marker test-message).user_message_matches) { throw 'queue_lf_rejected' }
    $turn.items[0].content[0].text += "`n"
    if ((Test-DesktopProbeReply $state test-thread $marker test-message).user_message_matches) { throw 'arbitrary_whitespace_accepted' }
    $turn.items[0].content[0].text = "Reply with exactly MARKER. Do not use any tools.`n"
    if ((Test-DesktopProbeReply $state test-thread MARKER test-message).user_message_matches) { throw 'direct_input_weakened' }
}
Check 'concurrent queue completion writes a single v3 frame without waiting for a reply' {
    $stream = [ProbeTranscriptStream]::new([byte[]]::new(0))
    $stream.Stall = $true
    try {
        Send-DesktopProbeQueueFrame $stream (QueueParameters) probe-client owner -RequestId $requestId -AllowTestQueue
        $bytes = $stream.Written.ToArray()
        if ($bytes.Length -ne 4 + [BitConverter]::ToUInt32($bytes,0)) { throw 'queue_multiple_frames_sent' }
        $sent = [Text.Encoding]::UTF8.GetString($bytes,4,$bytes.Length-4) | ConvertFrom-Json
        if ($sent.version -ne 3 -or $sent.method -cne 'thread-follower-set-queued-follow-ups-state' -or
            $sent.requestId -cne $requestId.ToString()) { throw 'queue_completion_wire_invalid' }
    } finally { $stream.Dispose() }
}
Check 'queue-only completion acknowledgements cannot be confused with direct start replies' {
    $reply = Reply 'thread-follower-start-turn' @{ ok = $true }
    $reply.handledByClientId = 'owner'
    $stream = [ProbeTranscriptStream]::new((Frame $reply))
    try {
        $rejected = $false
        try { $null = Receive-DesktopProbeTestAcknowledgement @{ pipe = $stream; owner = 'owner' } $requestId `
            -ExpectedMethod thread-follower-set-queued-follow-ups-state }
        catch { $rejected = $_.Exception.Message -ceq 'response_owner_mismatch' }
        if (-not $rejected) { throw 'direct_ack_misread_as_queue_ack' }
    } finally { $stream.Dispose() }
}
Check 'snapshot refresh preserves queue broadcasts for convergence checks' {
    $queue = @{ type = 'broadcast'; method = 'thread-queued-followups-changed'; version = 3
        sourceClientId = 'owner'; params = @{ hostId = 'local'; conversationId = 'test-thread'; messages = @() } }
    $snapshot = @{ type = 'broadcast'; method = 'thread-stream-state-changed'; version = 13; sourceClientId = 'owner'
        params = @{ hostId = 'local'; conversationId = 'test-thread'; change = @{ type = 'snapshot'; revision = 2; conversationState = (TestState) } } }
    $stream = [ProbeTranscriptStream]::new([byte[]]((Frame $queue) + (Frame $snapshot)))
    $seen = [Collections.Generic.List[object]]::new()
    try {
        $null = Receive-DesktopProbeSnapshot $stream probe-client owner test-thread -ExpectedVersion 13 -OnBroadcast {
            param($event)
            if ($event.method -ceq 'thread-queued-followups-changed') { $seen.Add($event.params.messages) }
        }
        if ($seen.Count -ne 1 -or $seen[0].Count -ne 0) { throw 'queue_clear_broadcast_lost' }
    } finally { $stream.Dispose() }
}
Write-Output "$passed protocol checks passed"
