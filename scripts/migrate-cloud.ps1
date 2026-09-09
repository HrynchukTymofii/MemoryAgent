<#
.SYNOPSIS
    Apply cloud/migrations/*.sql to the Neon database.

.DESCRIPTION
    Uses DATABASE_URL from .env — a *developer* credential. It is the direct
    Postgres connection string, it is never compiled into the app and never
    reaches a user's machine; the app authenticates as the signed-in user
    through the Data API instead, and holds no connection string at all.

    Falls back to printing the files when psql is not installed, because the
    Neon console's SQL Editor accepts exactly the same text.
#>
[CmdletBinding()]
param([switch]$Print)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$migrations = Get-ChildItem (Join-Path $root 'cloud/migrations/*.sql') | Sort-Object Name

if (-not $migrations) { Write-Error 'No migrations found.'; exit 1 }

$envFile = Join-Path $root '.env'
$url = $env:DATABASE_URL
if (-not $url -and (Test-Path $envFile)) {
    foreach ($line in Get-Content $envFile) {
        if ($line -match '^\s*DATABASE_URL\s*=\s*(.+?)\s*$') {
            $url = $Matches[1].Trim('"').Trim("'")
        }
    }
}

$psql = (Get-Command psql -ErrorAction SilentlyContinue)

if ($Print -or -not $psql -or -not $url) {
    if (-not $psql) { Write-Host 'psql not found.' -ForegroundColor Yellow }
    if (-not $url)  { Write-Host 'DATABASE_URL not set in .env.' -ForegroundColor Yellow }
    Write-Host ''
    Write-Host 'Paste the following into the Neon console SQL Editor:' -ForegroundColor Cyan
    foreach ($m in $migrations) {
        Write-Host ''
        Write-Host ('-- ' + $m.Name) -ForegroundColor DarkGray
        Get-Content $m.FullName -Raw
    }
    exit 0
}

foreach ($m in $migrations) {
    Write-Host "applying $($m.Name)..." -ForegroundColor Cyan
    & $psql.Source $url -v ON_ERROR_STOP=1 -f $m.FullName
    if ($LASTEXITCODE -ne 0) { Write-Error "failed on $($m.Name)"; exit 1 }
}
Write-Host 'done.' -ForegroundColor Green
