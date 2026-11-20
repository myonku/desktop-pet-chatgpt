# Private IPC experiment helpers. Turn writes require a separate explicit switch.
function New-DesktopProbeTestTurn {
    param([string] $ConversationId, [string] $Marker, [string] $MessageId)
    # The desktop owner renders the input before App Server normalization.
    # Its text helpers require text_elements, including for plain text.
    return @{ conversationId = $ConversationId
        turnStart = @{ request = @{ threadId = $ConversationId
            input = @(@{ type = 'text'; text = "Reply with exactly $Marker. Do not use any tools."; text_elements = @() })
            clientUserMessageId = $MessageId }; context = @{ inheritThreadSettings = $true } } }
}

function Write-DesktopProbeFrame {
    param([IO.Stream] $Stream, [System.Collections.IDictionary] $Message,
        [Threading.CancellationToken] $CancellationToken)
    $payload = [Text.Encoding]::UTF8.GetBytes(($Message | ConvertTo-Json -Compress -Depth 8))
    if ($payload.Length -gt 1MB) { throw 'invalid_frame_size' }
    $frame = [byte[]]([BitConverter]::GetBytes([uint32]$payload.Length) + $payload)
    [void]$Stream.WriteAsync($frame, 0, $frame.Length, $CancellationToken).GetAwaiter().GetResult()
}

function Read-DesktopProbeFrame {
    param([IO.Stream] $Stream, [Threading.CancellationToken] $CancellationToken)
    $prefix = [byte[]]::new(4)
    [void]$Stream.ReadExactlyAsync($prefix, 0, 4, $CancellationToken).AsTask().GetAwaiter().GetResult()
    $size = [BitConverter]::ToUInt32($prefix, 0)
    if ($size -lt 1 -or $size -gt 1MB) { throw 'invalid_frame_size' }
    $body = [byte[]]::new($size)
    [void]$Stream.ReadExactlyAsync($body, 0, $size, $CancellationToken).AsTask().GetAwaiter().GetResult()
    $reply = ([Text.UTF8Encoding]::new($false, $true)).GetString($body) |
        ConvertFrom-Json -AsHashtable -Depth 32
    if ($reply -isnot [System.Collections.IDictionary]) { throw 'invalid_frame' }
    return $reply
}

function Invoke-DesktopProbeControl {
    param([IO.Stream] $Stream, [System.Collections.IDictionary] $Message,
        [Threading.CancellationToken] $CancellationToken)
    if ($Message.type -ceq 'client-discovery-request') {
        if ($Message.requestId -isnot [string]) { throw 'invalid_discovery_request' }
        Write-DesktopProbeFrame $Stream @{
            type = 'client-discovery-response'; requestId = $Message.requestId
            response = @{ canHandle = $false }
        } $CancellationToken
        return $true
    }
    if ($Message.type -ceq 'request') {
        if ($Message.requestId -isnot [string]) { throw 'invalid_server_request' }
        Write-DesktopProbeFrame $Stream @{
            type = 'response'; requestId = $Message.requestId
            resultType = 'error'; error = 'no-handler-for-request'
        } $CancellationToken
        return $true
    }
    return $false
}

function Invoke-DesktopProbeRequest {
    param([IO.Stream] $Stream,
        [ValidateSet('initialize', 'thread-owner-discovery', 'thread-follower-start-turn', 'thread-follower-interrupt-turn',
            'thread-follower-set-queued-follow-ups-state', 'thread-follower-remove-queued-message')][string] $Method,
        [System.Collections.IDictionary] $Parameters, [string] $Source,
        [guid] $RequestId = [guid]::NewGuid(), [int] $TimeoutMs = 5000,
        [string] $Target, [switch] $AllowTestTurn, [switch] $AllowTestInterrupt,
        [switch] $AllowTestQueue, [switch] $AllowTestQueueRemoval, [scriptblock] $OnBroadcast)
    if ($Method -ceq 'thread-follower-start-turn' -and (-not $AllowTestTurn -or -not $Target)) {
        throw 'turn_write_not_enabled'
    }
    if ($Method -ceq 'thread-follower-interrupt-turn' -and (-not $AllowTestInterrupt -or -not $Target -or
        $Parameters.mode -cne 'user-stop' -or $Parameters.expectedTurnId -isnot [string] -or -not $Parameters.expectedTurnId)) {
        throw 'turn_interrupt_not_enabled'
    }
    if ($Method -ceq 'thread-follower-set-queued-follow-ups-state') {
        if (-not $AllowTestQueue -or -not $Target) { throw 'test_queue_not_enabled' }
        Assert-DesktopProbeQueueRequest $Parameters
    }
    if ($Method -ceq 'thread-follower-remove-queued-message' -and (-not $AllowTestQueueRemoval -or -not $Target -or
        $Parameters.messageId -cnotmatch '^[a-f0-9-]{36}\z')) { throw 'test_queue_removal_not_enabled' }
    $deadline = [Threading.CancellationTokenSource]::new($TimeoutMs)
    try {
        $request = @{
            type = 'request'; method = $Method
            version = @{ initialize = 0; 'thread-owner-discovery' = 1; 'thread-follower-start-turn' = 2; 'thread-follower-interrupt-turn' = 4
                'thread-follower-set-queued-follow-ups-state' = 3; 'thread-follower-remove-queued-message' = 1 }[$Method]
            params = $Parameters; requestId = $RequestId.ToString(); sourceClientId = $Source
            timeoutMs = [Math]::Max(1, $TimeoutMs - 1000)
        }
        if ($Target) { $request.targetClientId = $Target }
        Write-DesktopProbeFrame $Stream $request $deadline.Token
        for ($i = 0; $i -lt 64; $i++) {
            $reply = Read-DesktopProbeFrame $Stream $deadline.Token
            if ($reply.type -ceq 'response' -and $reply.requestId -ceq $RequestId.ToString()) {
                if ($reply.resultType -ceq 'success' -and
                    ($reply.method -cne $Method -or $reply.result -isnot [System.Collections.IDictionary])) {
                    throw 'invalid_response'
                }
                if ($reply.resultType -cnotin @('success', 'error')) { throw 'invalid_response' }
                if ($reply.resultType -ceq 'success' -and $Target -and $reply.handledByClientId -cne $Target) {
                    throw 'response_owner_mismatch'
                }
                return $reply
            }
            # A connected probe never claims ownership or handles desktop commands.
            # Reply promptly so other clients' discovery does not wait for our timeout.
            if ($reply.type -ceq 'broadcast' -and $OnBroadcast) {
                & $OnBroadcast $reply
            } else { $null = Invoke-DesktopProbeControl $Stream $reply $deadline.Token }
        }
        throw 'unmatched_frame_limit'
    } finally {
        $deadline.Dispose()
    }
}

function Send-DesktopProbeFollowing {
    param([IO.Stream] $Stream, [string] $Source, [string] $Owner, [string] $ConversationId,
        [bool] $Following)
    $deadline = [Threading.CancellationTokenSource]::new(5000)
    try {
        Write-DesktopProbeFrame $Stream @{
            type = 'broadcast'; method = 'thread-stream-following-changed'; version = 1
            sourceClientId = $Source; targetClientIds = @($Owner)
            params = @{ conversationId = $ConversationId; hostId = 'local'; following = $Following }
        } $deadline.Token
    } finally { $deadline.Dispose() }
}

function Receive-DesktopProbeSnapshot {
    param([IO.Stream] $Stream, [string] $Source, [string] $Owner, [string] $ConversationId,
        [int] $TimeoutMs = 5000, [ValidateRange(1, 100)][int] $ExpectedVersion = 11, [scriptblock] $OnBroadcast)
    $deadline = [Threading.CancellationTokenSource]::new($TimeoutMs)
    try {
        for ($i = 0; $i -lt 64; $i++) {
            $reply = Read-DesktopProbeFrame $Stream $deadline.Token
            if (Invoke-DesktopProbeControl $Stream $reply $deadline.Token) { continue }
            if ($reply.type -cne 'broadcast') { continue }
            if ($null -ne $reply.targetClientIds -and $Source -cnotin $reply.targetClientIds) { continue }
            if ($OnBroadcast) { & $OnBroadcast $reply }
            if ($reply.method -ceq 'client-status-changed' -and
                $reply.params.clientId -ceq $Owner -and $reply.params.status -ceq 'disconnected') {
                throw 'owner_disconnected'
            }
            if ($reply.method -cne 'thread-stream-state-changed' -or
                $reply.sourceClientId -cne $Owner -or $reply.params.hostId -cne 'local' -or
                $reply.params.conversationId -cne $ConversationId) { continue }
            if ($reply.version -cne $ExpectedVersion) {
                $error = [InvalidOperationException]::new('unsupported_stream_version')
                if ($reply.version -is [long] -or $reply.version -is [int]) {
                    $error.Data['stream_version'] = $reply.version
                }
                throw $error
            }
            if ($reply.params.change -isnot [System.Collections.IDictionary]) { throw 'invalid_stream_change' }
            if ($reply.params.change.type -ceq 'snapshot') { return $reply.params.change }
            if ($reply.params.change.type -cne 'patches') { throw 'unsupported_stream_change' }
        }
        throw 'unmatched_frame_limit'
    } finally { $deadline.Dispose() }
}

function Get-DesktopProbeTurns {
    param([System.Collections.IDictionary] $State)
    if ($State.turnHistory.history.entitiesByKey -is [System.Collections.IDictionary]) {
        return @($State.turnHistory.history.entitiesByKey.Values)
    }
    if ($null -ne $State.turnHistory) { throw 'unsupported_history_shape' }
    if ($State.turns -is [array]) { return $State.turns }
    throw 'unsupported_history_shape'
}

function Set-DesktopProbePatchValue {
    param($Node, [object[]] $Path, [int] $Depth, [System.Collections.IDictionary] $Patch)
    if ($Depth -eq $Path.Count) {
        if ($Patch.op -cne 'replace' -or -not $Patch.Contains('value')) { throw 'invalid_state_patch' }
        return ,$Patch.value
    }
    $key = $Path[$Depth]
    $leaf = $Depth -eq $Path.Count - 1
    if ($Node -is [System.Collections.IDictionary]) {
        if ($key -isnot [string]) { throw 'invalid_state_patch' }
        if (-not $leaf) {
            if (-not $Node.Contains($key)) { throw 'invalid_state_patch' }
            $Node[$key] = Set-DesktopProbePatchValue $Node[$key] $Path ($Depth + 1) $Patch
        } else {
            if ($Patch.op -cne 'add' -and -not $Node.Contains($key)) { throw 'invalid_state_patch' }
            # JSON omits an Immer value of undefined. For objects this must stay
            # absent when reserialized; for array slots it becomes null.
            if ($Patch.op -ceq 'remove' -or -not $Patch.Contains('value')) { $Node.Remove($key) }
            else { $Node[$key] = $Patch.value }
        }
        return ,$Node
    }
    if ($Node -is [array]) {
        if ($key -isnot [int] -and $key -isnot [long]) { throw 'invalid_state_patch' }
        $index = [int]$key
        if ($index -lt 0 -or $index -gt $Node.Count -or
            ($index -eq $Node.Count -and (-not $leaf -or $Patch.op -cne 'add'))) { throw 'invalid_state_patch' }
        if (-not $leaf) {
            $Node[$index] = Set-DesktopProbePatchValue $Node[$index] $Path ($Depth + 1) $Patch
            return ,$Node
        }
        $items = [Collections.Generic.List[object]]::new()
        foreach ($item in $Node) { $items.Add($item) }
        switch -CaseSensitive ($Patch.op) {
            'add' { $items.Insert($index, $Patch.value) }
            'replace' { $items[$index] = $Patch.value }
            'remove' { $items.RemoveAt($index) }
        }
        return ,($items.ToArray())
    }
    throw 'invalid_state_patch'
}

function Update-DesktopProbeStream {
    param([System.Collections.IDictionary] $Current, [System.Collections.IDictionary] $Change,
        [string] $ConversationId)
    if ($Change.type -ceq 'snapshot') {
        if (($Change.revision -isnot [int] -and $Change.revision -isnot [long]) -or
            $Change.conversationState -isnot [System.Collections.IDictionary]) { throw 'invalid_stream_change' }
        if ($Change.conversationState.id -cne $ConversationId -or $Change.conversationState.hostId -cne 'local') {
            throw 'snapshot_identity_mismatch'
        }
        if ($Change.revision -le $Current.revision) { return $false }
        $null = Get-DesktopProbeTurns $Change.conversationState
        $Current.conversationState = $Change.conversationState
        $Current.revision = $Change.revision
        return $true
    }
    if ($Change.type -cne 'patches' -or $Change.patches -isnot [array] -or
        ($Change.baseRevision -isnot [int] -and $Change.baseRevision -isnot [long]) -or
        ($Change.revision -isnot [int] -and $Change.revision -isnot [long]) -or
        $Change.revision -le $Change.baseRevision) { throw 'invalid_state_patch' }
    if ($Change.revision -le $Current.revision) { return $false }
    if ($Change.baseRevision -ne $Current.revision) { throw 'stream_revision_gap' }
    # Work on a copy so a rejected patch cannot partially advance accepted state.
    $next = $Current.conversationState | ConvertTo-Json -Depth 32 -Compress | ConvertFrom-Json -Depth 32 -AsHashtable
    foreach ($patch in $Change.patches) {
        if ($patch -isnot [System.Collections.IDictionary] -or $patch.op -cnotin @('add', 'replace', 'remove') -or
            $patch.path -isnot [array]) {
            throw 'invalid_state_patch'
        }
        try { $next = Set-DesktopProbePatchValue $next $patch.path 0 $patch }
        catch {
            $_.Exception.Data['patch_op'] = $patch.op
            $_.Exception.Data['patch_path_types'] = @($patch.path | ForEach-Object { $_.GetType().Name }) -join '/'
            throw
        }
    }
    if ($next.id -cne $ConversationId -or $next.hostId -cne 'local') { throw 'snapshot_identity_mismatch' }
    $Current.conversationState = $next
    $Current.revision = $Change.revision
    return $true
}

function Test-DesktopProbeReply {
    param([System.Collections.IDictionary] $State, [string] $ConversationId,
        [string] $Marker, [string] $MessageId)
    if ($State.id -cne $ConversationId -or $State.hostId -cne 'local') { throw 'snapshot_identity_mismatch' }
    $turns = @(Get-DesktopProbeTurns $State)
    $matches = @($turns | Where-Object { $_.params.clientUserMessageId -ceq $MessageId })
    if ($matches.Count -ne 1 -or $matches[0].params.threadId -cne $ConversationId) { throw 'test_turn_not_found' }
    $selected = $matches[0]
    $agents = @($selected.items | Where-Object { $_.type -ceq 'agentMessage' })
    $users = @($selected.items | Where-Object { $_.type -ceq 'userMessage' })
    $expectedText = "Reply with exactly $Marker. Do not use any tools."
    $userText = if ($users.Count -eq 1) { @($users[0].content | ForEach-Object { $_.text }) -join '' } else { $null }
    $userMatches = $users.Count -eq 1 -and ($userText -ceq $expectedText -or
        ($Marker -cmatch '^DESKTOP_PET_M6_QUEUE_[a-f0-9]{32}\z' -and $userText -ceq ($expectedText + "`n")))
    $result = @{
        thread_id_matches = $true; client_message_id_matches = $true
        completed = $selected.status -ceq 'completed'
        user_message_matches = $userMatches
        reply_matches = $agents.Count -eq 1 -and $agents[0].text -ceq $Marker
        no_tool_items = @($selected.items | Where-Object { $_.type -cnotin @('userMessage', 'agentMessage', 'reasoning') }).Count -eq 0
    }
    return $result
}

function Assert-DesktopProbeTestWriteReady {
    param([System.Collections.IDictionary] $State)
    if ($State.threadRuntimeStatus.type -cne 'idle' -or
        $State.turnHistory.history.isComplete -cne $true) { throw 'test_thread_not_ready' }
    $turns = @(Get-DesktopProbeTurns $State)
    # Runtime can be idle while a failed concurrent submission leaves an
    # optimistic in-progress row without a server turn ID. Do not send into it.
    if (@($turns | Where-Object { $_.status -ceq 'inProgress' }).Count -or
        @($State.unconfirmedTurnSubmissions).Where({ $null -ne $_ }).Count) { throw 'test_thread_not_ready' }
    foreach ($turn in $turns) {
        foreach ($input in $turn.params.input) {
            if ($input.type -ceq 'text' -and $input.text -cmatch 'DESKTOP_PET_M6_(?:IPC|QUEUE)_[a-f0-9]{32}') {
                throw 'test_write_already_attempted'
            }
        }
    }
}

function Test-DesktopProbeClientReply {
    param([System.Collections.IDictionary] $State, [string] $ConversationId,
        [string] $Marker = 'DESKTOP_PET_M6_CLIENT_OK')
    if ($State.id -cne $ConversationId -or $State.hostId -cne 'local') { throw 'snapshot_identity_mismatch' }
    $marker = $Marker
    $matches = @(Get-DesktopProbeTurns $State | Where-Object {
        $turn = $_
        $agents = @($turn.items | Where-Object { $_.type -ceq 'agentMessage' -and $_.text -ceq $marker })
        $users = @($turn.items | Where-Object { $_.type -ceq 'userMessage' -and
            (@($_.content | ForEach-Object { $_.text }) -join '').Contains($marker) })
        $turn.params.threadId -ceq $ConversationId -and $agents.Count -eq 1 -and $users.Count -eq 1
    })
    return @{ matching_turns = $matches.Count; reply_matches = $matches.Count -eq 1
        completed = $matches.Count -eq 1 -and $matches[0].status -ceq 'completed'
        no_tool_items = $matches.Count -eq 1 -and
            @($matches[0].items | Where-Object { $_.type -cnotin @('userMessage', 'agentMessage', 'reasoning') }).Count -eq 0 }
}

function Get-DesktopProbeStopTarget {
    param([System.Collections.IDictionary] $State, [string] $ConversationId, [string] $Marker)
    if ($State.id -cne $ConversationId -or $State.hostId -cne 'local') { throw 'snapshot_identity_mismatch' }
    if ($Marker -cnotmatch '^DESKTOP_PET_M6_CLIENT_STOP_[A-Z0-9_]+\z') { throw 'invalid_stop_marker' }
    $matches = @(Get-DesktopProbeTurns $State | Where-Object {
        $_.params.threadId -ceq $ConversationId -and $_.status -ceq 'inProgress' -and
        $_.turnId -is [string] -and $_.turnId -and
        @($_.params.input | Where-Object { $_.type -ceq 'text' -and $_.text.Contains($Marker) }).Count -gt 0
    })
    if ($matches.Count -eq 1 -and $State.threadRuntimeStatus.type -ceq 'active') {
        if (@($matches[0].items | Where-Object { $_.type -cnotin @('userMessage', 'agentMessage', 'reasoning') }).Count) {
            throw 'stop_test_has_tools'
        }
        return $matches[0].turnId
    }
    return $null
}

function Open-DesktopProbeSession {
    param([string] $ConversationId, [string] $ExpectedDesktopVersion = '26.1111.2228.0', [string] $OwnerHint)
    $packages = @(Get-AppxPackage OpenAI.Codex)
    if ($packages.Count -ne 1 -or $packages[0].PackageFamilyName -cne 'OpenAI.Codex_2p2nqsd0c76g0' -or
        $packages[0].Version.ToString() -cne $ExpectedDesktopVersion) { throw 'unsupported_desktop_package' }
    $manifest = Get-AppxPackageManifest -Package $packages[0].PackageFullName
    $app = $manifest.SelectSingleNode('//*[local-name()="Application" and @Id="App"]')
    if (-not $app.Executable) { throw 'desktop_executable_missing' }
    $image = [IO.Path]::GetFullPath((Join-Path $packages[0].InstallLocation $app.Executable))
    if (-not ('M6RecoveryPipe' -as [type])) {
        Add-Type -TypeDefinition @'
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;
public static class M6RecoveryPipe {
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool GetNamedPipeServerProcessId(SafePipeHandle pipe, out int pid);
}
'@
    }
    $pipe = [IO.Pipes.NamedPipeClientStream]::new('.', 'codex-ipc', [IO.Pipes.PipeDirection]::InOut,
        [IO.Pipes.PipeOptions]::Asynchronous, [Security.Principal.TokenImpersonationLevel]::Identification)
    try {
        $pipe.Connect(2000)
        [int]$serverId = 0
        if (-not [M6RecoveryPipe]::GetNamedPipeServerProcessId($pipe.SafePipeHandle,[ref]$serverId)) { throw 'pipe_peer_unknown' }
        $server = Get-Process -Id $serverId
        if ($server.SessionId -ne (Get-Process -Id $PID).SessionId -or
            -not [string]::Equals($server.Path,$image,[StringComparison]::OrdinalIgnoreCase)) { throw 'unexpected_pipe_peer' }
        $reply = Invoke-DesktopProbeRequest $pipe initialize @{ clientType = 'bailong' } initializing-client
        $client = $reply.result.clientId
        if ($reply.resultType -cne 'success' -or $client -isnot [string] -or
            $client -cnotmatch '^[A-Za-z0-9][A-Za-z0-9._:-]{0,199}\z') { throw 'initialize_not_confirmed' }
        if (-not $OwnerHint) {
            $negative = Invoke-DesktopProbeRequest $pipe thread-owner-discovery @{
                hostId = 'local'; conversationId = [guid]::NewGuid().ToString() } $client
            if ($negative.resultType -cne 'error' -or $negative.error -cne 'no-client-found') { throw 'unexpected_discovery' }
        }
        # The second prepared writer has no reader yet. Use the verified owner's
        # directed discovery instead of making the first writer answer a broadcast.
        $routing = @{}
        if ($OwnerHint) { $routing.Target = $OwnerHint }
        $reply = Invoke-DesktopProbeRequest $pipe thread-owner-discovery @{ hostId = 'local'; conversationId = $ConversationId } $client @routing
        if ($reply.resultType -cne 'success' -or $reply.handledByClientId -isnot [string] -or
            $reply.handledByClientId -cnotmatch '^[A-Za-z0-9][A-Za-z0-9._:-]{0,199}\z') { throw 'codex_owner_required' }
        return @{ pipe = $pipe; client = $client; owner = $reply.handledByClientId; conversation = $ConversationId
            desktopVersion = $ExpectedDesktopVersion; peerGeneration = "$serverId/$($server.StartTime.Ticks)" }
    } catch { $pipe.Dispose(); throw }
}

function Close-DesktopProbeSession {
    param([System.Collections.IDictionary] $Session, [switch] $Abrupt)
    if (-not $Session) { return }
    if (-not $Abrupt) {
        try { Send-DesktopProbeFollowing $Session.pipe $Session.client $Session.owner $Session.conversation $false } catch { }
    }
    $Session.pipe.Dispose()
}

function Get-DesktopProbeSessionSnapshot {
    param([System.Collections.IDictionary] $Session, [scriptblock] $OnQueue)
    Send-DesktopProbeFollowing $Session.pipe $Session.client $Session.owner $Session.conversation $true
    $snapshot = Receive-DesktopProbeSnapshot $Session.pipe $Session.client $Session.owner $Session.conversation -ExpectedVersion 13 -OnBroadcast $OnQueue
    if ($snapshot.conversationState.id -cne $Session.conversation -or $snapshot.conversationState.hostId -cne 'local' -or
        ($snapshot.revision -isnot [int] -and $snapshot.revision -isnot [long])) { throw 'snapshot_identity_mismatch' }
    $null = Get-DesktopProbeTurns $snapshot.conversationState
    return $snapshot
}

function Receive-DesktopProbeSessionChange {
    param([System.Collections.IDictionary] $Session, [Threading.CancellationToken] $CancellationToken, [scriptblock] $OnQueue)
    for ($i = 0; $i -lt 2048; $i++) {
        $event = Read-DesktopProbeFrame $Session.pipe $CancellationToken
        if (Invoke-DesktopProbeControl $Session.pipe $event $CancellationToken) { continue }
        if ($event.type -cne 'broadcast' -or
            ($null -ne $event.targetClientIds -and $Session.client -cnotin $event.targetClientIds)) { continue }
        if (($event.method -ceq 'client-status-changed' -and $event.params.clientId -ceq $Session.owner -and
            $event.params.status -ceq 'disconnected') -or $event.method -ceq 'ipc-connection-reset') { throw 'owner_disconnected' }
        if ($event.method -ceq 'thread-queued-followups-changed' -and $event.sourceClientId -ceq $Session.owner -and
            $event.params.hostId -ceq 'local' -and $event.params.conversationId -ceq $Session.conversation -and $OnQueue) {
            if ($event.version -cne 3) { throw 'unsupported_queue_version' }
            & $OnQueue $event
            continue
        }
        if ($event.method -cne 'thread-stream-state-changed' -or $event.sourceClientId -cne $Session.owner -or
            $event.params.hostId -cne 'local' -or $event.params.conversationId -cne $Session.conversation) { continue }
        if ($event.version -cne 13) { throw 'unsupported_stream_version' }
        if ($event.params.change -isnot [System.Collections.IDictionary]) { throw 'invalid_stream_change' }
        return $event.params.change
    }
    throw 'unmatched_frame_limit'
}

function Send-DesktopProbeTestFrame {
    param([IO.Stream] $Stream, [string] $ConversationId, [string] $Marker, [string] $MessageId,
        [string] $Source, [string] $Owner, [guid] $RequestId = [guid]::NewGuid(), [switch] $AllowTestTurn)
    if (-not $AllowTestTurn -or -not $Owner -or -not $Source -or
        $Marker -cnotmatch '^DESKTOP_PET_M6_IPC_[a-f0-9]{32}\z' -or
        $MessageId -cnotmatch '^[a-f0-9-]{36}\z') { throw 'turn_write_not_enabled' }
    $deadline = [Threading.CancellationTokenSource]::new(5000)
    try {
        Write-DesktopProbeFrame $Stream @{ type = 'request'; method = 'thread-follower-start-turn'; version = 2
            params = (New-DesktopProbeTestTurn $ConversationId $Marker $MessageId)
            sourceClientId = $Source; targetClientId = $Owner; requestId = $RequestId.ToString(); timeoutMs = 29000
        } $deadline.Token
    } finally { $deadline.Dispose() }
}

function Receive-DesktopProbeTestAcknowledgement {
    param([System.Collections.IDictionary] $Session, [guid] $RequestId, [int] $TimeoutMs = 30000,
        [ValidateSet('thread-follower-start-turn','thread-follower-set-queued-follow-ups-state')]
        [string] $ExpectedMethod = 'thread-follower-start-turn', [scriptblock] $OnBroadcast)
    $deadline = [Threading.CancellationTokenSource]::new($TimeoutMs)
    try {
        for ($i = 0; $i -lt 512; $i++) {
            $reply = Read-DesktopProbeFrame $Session.pipe $deadline.Token
            if (Invoke-DesktopProbeControl $Session.pipe $reply $deadline.Token) { continue }
            if ($reply.type -ceq 'broadcast' -and $OnBroadcast) { & $OnBroadcast $reply; continue }
            if ($reply.type -cne 'response' -or $reply.requestId -cne $RequestId.ToString()) { continue }
            if ($reply.resultType -cnotin @('success','error')) { throw 'invalid_response' }
            if ($reply.resultType -ceq 'success' -and ($reply.method -cne $ExpectedMethod -or
                $reply.handledByClientId -cne $Session.owner -or $reply.result -isnot [System.Collections.IDictionary])) {
                throw 'response_owner_mismatch'
            }
            return $reply
        }
        throw 'unmatched_frame_limit'
    } finally { $deadline.Dispose() }
}

function New-DesktopProbeQueueRequest {
    param([System.Collections.IDictionary] $State, [string] $ConversationId, [string] $Marker,
        [string] $MessageId, [switch] $Reserve)
    if ($State.id -cne $ConversationId -or $State.hostId -cne 'local' -or $State.cwd -isnot [string] -or -not $State.cwd) {
        throw 'snapshot_identity_mismatch'
    }
    if ($Marker -cnotmatch '^DESKTOP_PET_M6_QUEUE_[a-f0-9]{32}\z' -or
        $MessageId -cnotmatch '^[a-f0-9-]{36}\z') { throw 'invalid_test_queue_request' }
    $text = "Reply with exactly $Marker. Do not use any tools."
    $message = @{ id = $MessageId; text = $text; cwd = $State.cwd; createdAt = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
        context = @{ prompt = $text; addedFiles = @(); fileAttachments = @(); imageAttachments = @(); ideContext = $null
            workspaceRoots = @($State.cwd) } }
    if ($Reserve) { $message.preparation = @{} }
    $stateMap = @{}
    $stateMap[$ConversationId] = @($message)
    return @{ conversationId = $ConversationId; state = $stateMap; discardedMessageIds = @(); preparationMessageId = $MessageId }
}

function Assert-DesktopProbeQueueRequest {
    param([System.Collections.IDictionary] $Parameters)
    $conversation = $Parameters.conversationId
    if ($conversation -isnot [string] -or $Parameters.state -isnot [System.Collections.IDictionary] -or
        $Parameters.state.Count -ne 1 -or -not $Parameters.state.Contains($conversation) -or
        $Parameters.state[$conversation] -isnot [array] -or $Parameters.state[$conversation].Count -ne 1 -or
        $Parameters.discardedMessageIds -isnot [array] -or $Parameters.discardedMessageIds.Count -ne 0 -or
        $Parameters.preparationMessageId -cnotmatch '^[a-f0-9-]{36}\z') { throw 'invalid_test_queue_request' }
    $message = $Parameters.state[$conversation][0]
    if ($message.id -cne $Parameters.preparationMessageId -or $message.text -isnot [string] -or
        $message.text -cnotmatch '^Reply with exactly DESKTOP_PET_M6_QUEUE_[a-f0-9]{32}\. Do not use any tools\.$' -or
        $message.context.prompt -cne $message.text -or $message.cwd -isnot [string] -or -not $message.cwd -or
        $message.context.addedFiles -isnot [array] -or $message.context.fileAttachments -isnot [array] -or
        $message.context.imageAttachments -isnot [array] -or $null -ne $message.context.ideContext -or
        $message.context.workspaceRoots -isnot [array] -or
        $message.context.addedFiles.Count -ne 0 -or $message.context.fileAttachments.Count -ne 0 -or
        $message.context.imageAttachments.Count -ne 0 -or $message.context.workspaceRoots.Count -ne 1 -or
        $message.context.workspaceRoots[0] -cne $message.cwd -or
        @($message.Keys | Where-Object { $_ -cnotin @('id','text','cwd','createdAt','context','preparation') }).Count -ne 0 -or
        @($message.context.Keys | Where-Object { $_ -cnotin @('prompt','addedFiles','fileAttachments','imageAttachments','ideContext','workspaceRoots') }).Count -ne 0 -or
        ($message.Contains('preparation') -and ($message.preparation -isnot [System.Collections.IDictionary] -or $message.preparation.Count -ne 0))) {
        throw 'invalid_test_queue_request'
    }
}

function Send-DesktopProbeQueueFrame {
    param([IO.Stream] $Stream, [System.Collections.IDictionary] $Parameters, [string] $Source, [string] $Owner,
        [guid] $RequestId, [switch] $AllowTestQueue)
    if (-not $AllowTestQueue -or -not $Source -or -not $Owner -or $RequestId -eq [guid]::Empty) { throw 'test_queue_not_enabled' }
    Assert-DesktopProbeQueueRequest $Parameters
    $deadline = [Threading.CancellationTokenSource]::new(5000)
    try {
        Write-DesktopProbeFrame $Stream @{ type = 'request'; method = 'thread-follower-set-queued-follow-ups-state'; version = 3
            params = $Parameters; sourceClientId = $Source; targetClientId = $Owner
            requestId = $RequestId.ToString(); timeoutMs = 14000 } $deadline.Token
    } finally { $deadline.Dispose() }
}

Export-ModuleMember -Function New-DesktopProbeTestTurn, Update-DesktopProbeStream, Invoke-DesktopProbeControl, Invoke-DesktopProbeRequest, Send-DesktopProbeFollowing, Receive-DesktopProbeSnapshot, Read-DesktopProbeFrame, Get-DesktopProbeTurns, Test-DesktopProbeReply, Assert-DesktopProbeTestWriteReady, Test-DesktopProbeClientReply, Get-DesktopProbeStopTarget, Open-DesktopProbeSession, Close-DesktopProbeSession, Get-DesktopProbeSessionSnapshot, Receive-DesktopProbeSessionChange, Send-DesktopProbeTestFrame, Receive-DesktopProbeTestAcknowledgement, New-DesktopProbeQueueRequest, Assert-DesktopProbeQueueRequest, Send-DesktopProbeQueueFrame
