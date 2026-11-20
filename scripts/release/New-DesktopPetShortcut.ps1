param(
    [Parameter(Mandatory)]
    [string]$Exe,
    [string]$Pack,
    [string]$Validator,
    [string]$Output = (Join-Path ([Environment]::GetFolderPath('DesktopDirectory')) 'Desktop Pet ChatGPT.lnk')
)

$ErrorActionPreference = 'Stop'
$exePath = (Get-Item -LiteralPath $Exe).FullName
$install = Split-Path -Parent $exePath
if (-not $Pack) { $Pack = Join-Path $install 'resources\default' }
$outputPath = [IO.Path]::GetFullPath($Output)
if ([IO.Path]::GetExtension($outputPath) -ine '.lnk' -or (Test-Path -LiteralPath $outputPath)) {
    throw 'Choose a new .lnk output path.'
}

$iconPath = $exePath
$manifestPath = Join-Path $Pack 'manifest.json'
if (Test-Path -LiteralPath $manifestPath -PathType Leaf) {
    $manifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding utf8 | ConvertFrom-Json
    $icon = $manifest.resources | Where-Object {
        $_.key -ceq 'app.icon' -and $null -eq $_.client_id -and $_.content.kind -ceq 'icon'
    } | Select-Object -First 1
    if ($icon) {
        if (-not $Validator) { throw 'Provide -Validator to verify the icon and its asset pack.' }
        $validatorPath = (Get-Item -LiteralPath $Validator).FullName
        $validation = & $validatorPath --distribution $Pack 2>&1
        if ($LASTEXITCODE -eq 0) {
            $iconPath = (Get-Item -LiteralPath (Join-Path $Pack $icon.content.file.path)).FullName
        } else {
            Write-Warning "Asset pack validation failed. Using the embedded application icon. $validation"
        }
    }
}

$shell = New-Object -ComObject WScript.Shell
$link = $shell.CreateShortcut($outputPath)
$link.TargetPath = $exePath
$link.WorkingDirectory = $install
$link.IconLocation = "$iconPath,0"
$link.Save()
Write-Output $outputPath
