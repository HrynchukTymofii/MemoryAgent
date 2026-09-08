# Build the Tier 1 router sidecar.
#
#   .\scripts\build-router.ps1            # release, next to the app binary
#   .\scripts\build-router.ps1 -Debug     # debug, for `tauri dev`
#
# Separate from the app's own build on purpose (ADR-0008). This is the only
# binary in the workspace that links llama.cpp, and llama.cpp cannot be linked
# into the app: whisper.cpp vendors its own copy of ggml, so one executable
# containing both defines every ggml symbol twice and fails to link.
#
# The app finds this binary beside its own, so both come out of target\<profile>
# and no path configuration is involved. Without it the app still runs - Tier 0
# answers the formulaic commands and unusual phrasings come back "not sure what
# to do with that", which is exactly the behaviour before Tier 1 existed.
#
# ASCII only, deliberately: Windows PowerShell 5.1 reads a .ps1 as ANSI unless
# it carries a BOM, so a stray em-dash or curly quote turns into mojibake and
# breaks the parse with an error that points nowhere near the real cause.

param([switch]$Debug)

$ErrorActionPreference = "Stop"

$root = Split-Path $PSScriptRoot -Parent
$profileName = if ($Debug) { "debug" } else { "release" }

Write-Host "Building memos-router ($profileName). First run compiles llama.cpp; expect a few minutes."

# cargo writes its progress to stderr, and Windows PowerShell wraps every stderr
# line as an ErrorRecord - so a perfectly healthy build would abort the script
# with NativeCommandError. Same reason fetch-models.ps1 handles curl this way.
$prev = $ErrorActionPreference
$ErrorActionPreference = "Continue"
if ($Debug) {
    & cargo build --manifest-path "$root\Cargo.toml" -p memos-llm --features local --bin memos-router
} else {
    & cargo build --manifest-path "$root\Cargo.toml" --release -p memos-llm --features local --bin memos-router
}
$code = $LASTEXITCODE
$ErrorActionPreference = $prev

if ($code -ne 0) {
    throw "Build failed (cargo exit $code). llama.cpp needs CMake and the MSVC build tools - see the Toolchain section of README.md."
}

$exe = Join-Path $root "target\$profileName\memos-router.exe"
if (-not (Test-Path $exe)) {
    throw "cargo reported success but $exe is not there."
}
$mb = [math]::Round((Get-Item $exe).Length / 1MB, 1)
Write-Host "Done: $exe ($mb MB)" -ForegroundColor Green

# Checked here rather than left to the app, because a router binary with no
# model is a silent no-op: the app starts, Tier 1 reports "missing", and the
# only symptom is that unusual phrasings keep failing.
$model = Join-Path $root "models\llm\router.gguf"
if (-not (Test-Path $model)) {
    Write-Host "No model yet. Fetch it: .\scripts\fetch-models.ps1 router" -ForegroundColor Yellow
}
