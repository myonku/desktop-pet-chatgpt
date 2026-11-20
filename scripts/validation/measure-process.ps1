param(
    [Parameter(Mandatory)]
    [ValidateNotNullOrEmpty()]
    [int[]]$ProcessIds,

    [ValidateRange(1, 3600)]
    [int]$DurationSeconds = 600,

    [ValidateRange(100, 10000)]
    [int]$IntervalMilliseconds = 1000,

    [Parameter(Mandatory)]
    [string]$OutputCsv
)

$ErrorActionPreference = 'Stop'
$processes = Get-Process -Id ($ProcessIds | Select-Object -Unique)
$clock = [System.Diagnostics.Stopwatch]::StartNew()
$previous = @{}

& {
    do {
        foreach ($process in $processes) {
            $process.Refresh()
            if ($process.HasExited) {
                throw "Process $($process.Id) exited during sampling."
            }

            $elapsed = $clock.Elapsed.TotalSeconds
            $cpu = $process.TotalProcessorTime.TotalSeconds
            $prior = $previous[$process.Id]
            $percent = if ($null -ne $prior -and $elapsed -gt $prior.Elapsed) {
                100 * ($cpu - $prior.Cpu) / ($elapsed - $prior.Elapsed)
            }
            $previous[$process.Id] = @{ Elapsed = $elapsed; Cpu = $cpu }

            [pscustomobject]@{
                ProcessId = $process.Id
                ParentProcessId = $process.Parent.Id
                ProcessName = $process.ProcessName
                ElapsedSeconds = $elapsed
                CpuSeconds = $cpu
                CpuPercentOneCore = $percent
                PrivateBytes = $process.PrivateMemorySize64
                WorkingSetBytes = $process.WorkingSet64
                PeakWorkingSetBytes = $process.PeakWorkingSet64
            }
        }
        if ($clock.Elapsed.TotalSeconds -ge $DurationSeconds) { break }
        Start-Sleep -Milliseconds $IntervalMilliseconds
    } while ($true)
} | Export-Csv -LiteralPath $OutputCsv -NoTypeInformation -NoClobber
