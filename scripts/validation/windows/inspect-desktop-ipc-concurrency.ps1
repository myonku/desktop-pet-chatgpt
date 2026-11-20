#Requires -Version 7.4
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidatePattern('^[A-Za-z0-9_-]{1,128}\z')][string] $CodexTestId,
    [Parameter(Mandatory)][string] $ReceiptDirectory,
    [Parameter(Mandatory)][string] $OutputDirectory
)
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot desktop_ipc.psm1) -Force
if (Test-Path -LiteralPath (Join-Path $OutputDirectory result.json)) { throw 'inspection_receipt_already_exists' }
$receipt = Get-Content -LiteralPath (Join-Path $ReceiptDirectory result.json) -Raw | ConvertFrom-Json -AsHashtable
$validPair = $receipt.probe_scope -ceq 'two_follower_writes' -and $receipt.write_attempts -eq 2
$validQueue = $receipt.probe_scope -ceq 'owner_queue_reserve_then_complete' -and $receipt.queue_mutation_attempts -eq 4
if ((-not $validPair -and -not $validQueue) -or $receipt.requests.Count -ne 2) { throw 'invalid_concurrency_receipt' }
foreach ($request in $receipt.requests) {
    if ($request.expected_test_marker -cnotmatch '^DESKTOP_PET_M6_(?:IPC|QUEUE)_[a-f0-9]{32}\z' -or
        $request.client_message_id -cnotmatch '^[a-f0-9-]{36}\z') { throw 'invalid_concurrency_receipt' }
}
$outputPath = (New-Item -ItemType Directory -Path $OutputDirectory -Force).FullName
$report = [ordered]@{ status = 'error'; probe_scope = 'concurrency_read_only_inspection'; write_attempts = 0; desktop_ui_verified = $false }
$session = $null
try {
    $session = Open-DesktopProbeSession $CodexTestId
    $snapshot = Get-DesktopProbeSessionSnapshot $session
    $report.desktop_peer_verified = $true
    $report.thread_runtime_status = $snapshot.conversationState.threadRuntimeStatus.type
    $rows = @()
    foreach ($turn in @(Get-DesktopProbeTurns $snapshot.conversationState)) {
        $paramsMatch = @()
        $userMatches = @()
        $agentMatches = @()
        $userTextForms = @()
        foreach ($index in 0..1) {
            $request = $receipt.requests[$index]
            if ($turn.params.clientUserMessageId -ceq $request.client_message_id) { $paramsMatch += $index }
            $users = @($turn.items | Where-Object { $_.type -ceq 'userMessage' -or $_.type -ceq 'steeringUserMessage' })
            foreach ($user in $users) {
                $text = @($user.content | ForEach-Object { $_.text }) -join ''
                $expected = "Reply with exactly $($request.expected_test_marker). Do not use any tools."
                if ($user.clientId -ceq $request.client_message_id -or $text.TrimEnd() -ceq $expected) {
                    $userTextForms += @{ request_index = $index; exact = $text -ceq $expected
                        appended_lf = $text -ceq ($expected + "`n"); appended_crlf = $text -ceq ($expected + "`r`n") }
                }
            }
            if (@($users | Where-Object {
                $_.clientId -ceq $request.client_message_id -or $_.clientUserMessageId -ceq $request.client_message_id -or
                (@($_.content | ForEach-Object { $_.text }) -join '') -ceq "Reply with exactly $($request.expected_test_marker). Do not use any tools."
            }).Count) { $userMatches += $index }
            if (@($turn.items | Where-Object { $_.type -ceq 'agentMessage' -and $_.text -ceq $request.expected_test_marker }).Count) { $agentMatches += $index }
        }
        if ($paramsMatch.Count -or $userMatches.Count -or $agentMatches.Count) {
            $rows += @{ turn_id = $turn.turnId; status = $turn.status; params_message_indexes = $paramsMatch
                user_message_indexes = $userMatches; agent_marker_indexes = $agentMatches
                item_types = @($turn.items | ForEach-Object { $_.type }); user_message_count = $users.Count }
            $rows[-1].user_text_forms = $userTextForms
        }
    }
    $report.correlated_turns = $rows
    $report.shared_ack_turn = $receipt.requests[0].ack_turn_id -and $receipt.requests[0].ack_turn_id -ceq $receipt.requests[1].ack_turn_id
    $report.status = 'ok'
} catch {
    $report.failure_type = $_.Exception.GetType().Name
    $report.failure_id = $_.FullyQualifiedErrorId
    $report.failure_line = $_.InvocationInfo.ScriptLineNumber
} finally {
    Close-DesktopProbeSession $session
    $report | ConvertTo-Json -Depth 7 | Tee-Object -FilePath (Join-Path $outputPath result.json)
}
if ($report.status -cne 'ok') { exit 1 }
