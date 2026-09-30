param(
    [Parameter(Mandatory = $true)][string]$Root,
    [string]$Exe = 'target/release/openomsi.exe',
    [ValidatePattern('^[a-zA-Z0-9_-]+$')][string]$Name = 'th-wald',
    [string]$Bus = 'Vehicles/MB_C2_EN_BVG/MB_C2_E6_Gn_BVG_main.bus',
    [ValidateSet('outside', 'driver')][string]$View = 'outside',
    [ValidateSet('auto', 'dx12', 'vulkan', 'gl')][string]$Backend = 'auto',
    [int]$Seconds = 100,
    [switch]$NoSplineBatching,
    [switch]$NoGroundSplineBatching,
    [switch]$NoMaterialSplineBatching,
    [switch]$NoBoundsCache,
    [switch]$NoRenderPool
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$output = Join-Path $repo 'target/performance-284'
New-Item -ItemType Directory -Path $output -Force | Out-Null
$binary = if ([IO.Path]::IsPathRooted($Exe)) { $Exe } else { Join-Path $repo $Exe }
$variables = @{
    RUST_LOG = 'info'
    OMSI_PROFILE = '1'
    OMSI_GPU_TIMERS = '1'
    OMSI_SEED = '30794024'
    OMSI_MAX_FPS = '0'
    OMSI_BACKEND = $(if ($Backend -eq 'auto') { $null } else { $Backend })
    OMSI_CONTENT = (Join-Path $output 'content')
    OMSI_INPUT = "t=80 shot $($output.Replace('\', '/'))/$Name.png"
    OMSI_NO_SPLINE_BATCHING = $(if ($NoSplineBatching) { '1' } else { $null })
    OMSI_NO_GROUND_SPLINE_BATCHING = $(if ($NoGroundSplineBatching) { '1' } else { $null })
    OMSI_NO_MATERIAL_SPLINE_BATCHING = $(if ($NoMaterialSplineBatching) { '1' } else { $null })
    OMSI_NO_BOUNDS_CACHE = $(if ($NoBoundsCache) { '1' } else { $null })
    OMSI_NO_RENDER_POOL = $(if ($NoRenderPool) { '1' } else { $null })
}
$previous = @{}
try {
    foreach ($key in $variables.Keys) {
        $previous[$key] = [Environment]::GetEnvironmentVariable($key, 'Process')
        if ($null -eq $variables[$key]) {
            Remove-Item -LiteralPath "Env:$key" -ErrorAction SilentlyContinue
        } else {
            [Environment]::SetEnvironmentVariable($key, $variables[$key], 'Process')
        }
    }
    $arguments = @(
        '--root', ('"{0}"' -f $Root), '--no-menu', '--map', 'maps/TH_Wald/global.cfg',
        '--bus', ('"{0}"' -f $Bus), '--time', '09:03', '--hof', '"Thueringer Wald 2005"',
        '--date', '2017-10-07', '--traffic', '20', '--passengers', '--schedule',
        '--season', 'autumn', '--spawn', '1792.7,1561.9,187,6.0', '--view', $View,
        '--exit-after', $Seconds
    )
    $process = Start-Process -FilePath $binary -ArgumentList $arguments -WorkingDirectory $repo `
        -WindowStyle Hidden -RedirectStandardOutput (Join-Path $output "$Name.stdout.log") `
        -RedirectStandardError (Join-Path $output "$Name.log") -PassThru
    Write-Output "PID $($process.Id); log: $output/$Name.log"
    $process.WaitForExit()
    if ($process.ExitCode -ne 0) { throw "Game exited with $($process.ExitCode)" }
} finally {
    foreach ($key in $variables.Keys) {
        if ($null -eq $previous[$key]) {
            Remove-Item -LiteralPath "Env:$key" -ErrorAction SilentlyContinue
        } else {
            [Environment]::SetEnvironmentVariable($key, $previous[$key], 'Process')
        }
    }
}
