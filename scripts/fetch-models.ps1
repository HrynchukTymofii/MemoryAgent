# Download the models the app needs.
#
#   .\scripts\fetch-models.ps1              # base.en  (~148 MB, the default)
#   .\scripts\fetch-models.ps1 tiny.en      # ~78 MB, for the Light profile
#   .\scripts\fetch-models.ps1 small.en     # ~488 MB, better accuracy
#   .\scripts\fetch-models.ps1 embedding    # ~33 MB, semantic search
#   .\scripts\fetch-models.ps1 router       # ~609 MB, the Tier 1 intent router
#   .\scripts\fetch-models.ps1 router-light # ~378 MB, same model, Q4
#   .\scripts\fetch-models.ps1 bundle      # stage what the installer ships
#
# Weights are never committed - .gitignore excludes *.bin, *.onnx and friends.
#
# ASCII only, deliberately: Windows PowerShell 5.1 reads a .ps1 as ANSI unless
# it carries a BOM, so a stray em-dash or curly quote turns into mojibake and
# breaks the parse with an error that points nowhere near the real cause.

param([string]$Model = "base.en")

$ErrorActionPreference = "Stop"

# Shared by both paths below.
#
# curl.exe rather than Invoke-WebRequest: IWR buffers the whole body in memory,
# which is painful for a half-gigabyte model and shows no progress.
# --silent, not --progress-bar: curl writes progress to stderr, and Windows
# PowerShell wraps every stderr line as an ErrorRecord, so a perfectly healthy
# download aborts the script with NativeCommandError.
function Get-File($Url, $Dest, $ExpectedMb) {
    if (Test-Path $Dest) {
        $have = [math]::Round((Get-Item $Dest).Length / 1MB)
        Write-Host "Already present: $Dest ($have MB)" -ForegroundColor Green
        return
    }
    Write-Host "Downloading $(Split-Path $Dest -Leaf) (about $ExpectedMb MB)"
    Write-Host "  from $Url"
    Write-Host "  to   $Dest"

    $curl = Get-Command curl.exe -ErrorAction SilentlyContinue
    if ($curl) {
        $prev = $ErrorActionPreference
        $ErrorActionPreference = "Continue"
        & $curl.Source -L --fail --silent --show-error -o $Dest $Url
        $code = $LASTEXITCODE
        $ErrorActionPreference = $prev
        if ($code -ne 0) {
            Remove-Item $Dest -ErrorAction SilentlyContinue
            throw "Download failed (curl exit $code)"
        }
    } else {
        $ProgressPreference = "SilentlyContinue"
        Invoke-WebRequest -Uri $Url -OutFile $Dest -UseBasicParsing
    }

    # A truncated model loads and then produces garbage, so check the size
    # rather than trusting that the transfer finished.
    $have = [math]::Round((Get-Item $Dest).Length / 1MB)
    if ($have -lt ($ExpectedMb * 0.5)) {
        Remove-Item $Dest -Force
        throw "Downloaded file is only $have MB, expected about $ExpectedMb MB. Removed the partial file."
    }
    Write-Host "Done: $Dest ($have MB)" -ForegroundColor Green
}

# What goes inside the installer.
#
# The point of this target is that a person who installs the app never runs a
# script. tiny.en rather than base.en: 78 MB against 148 MB, and it is the floor
# that makes the product work on first launch, not the ceiling - base.en is
# offered later as an upgrade and wins over this one wherever both exist.
#
# The ONNX Runtime is shipped as well, and it is the one that is not optional to
# bundle. It is a library the process dlopens, and on macOS the hardened runtime
# only lets a signed process load code signed by the same team - so it has to be
# signed with the app rather than downloaded afterwards.
#
# The layout below is the one the app looks for; see bundle_dir() in main.rs.
if ($Model -eq "bundle") {
    & $PSCommandPath tiny.en
    & $PSCommandPath embedding

    $root = Join-Path $PSScriptRoot ".."
    $staging = Join-Path $root "apps\desktop\src-tauri\resources"
    New-Item -ItemType Directory -Force -Path (Join-Path $staging "models\embedding") | Out-Null
    New-Item -ItemType Directory -Force -Path (Join-Path $staging "runtime") | Out-Null

    $pairs = @(
        @("models\stt\ggml-tiny.en.bin",     "models\ggml-tiny.en.bin"),
        @("models\embedding\model.onnx",     "models\embedding\model.onnx"),
        @("models\embedding\tokenizer.json", "models\embedding\tokenizer.json"),
        @("runtime\onnxruntime.dll",         "runtime\onnxruntime.dll"),
        @("runtime\onnxruntime_providers_shared.dll", "runtime\onnxruntime_providers_shared.dll")
    )
    foreach ($pair in $pairs) {
        $src = Join-Path $root $pair[0]
        $dst = Join-Path $staging $pair[1]
        if (-not (Test-Path $src)) { throw "Missing $src - fetch it first." }
        Copy-Item $src $dst -Force
        $mb = [math]::Round((Get-Item $dst).Length / 1MB)
        Write-Host "Staged: $($pair[1]) ($mb MB)" -ForegroundColor Green
    }

    $total = [math]::Round(((Get-ChildItem $staging -Recurse -File | Measure-Object Length -Sum).Sum) / 1MB)
    Write-Host ""
    Write-Host "Staged $total MB. 'npm run tauri build' now ships it." -ForegroundColor Green
    exit 0
}

# The embedding model: bge-small-en-v1.5, int8-quantized ONNX export.
#
# Quantized, at a third of the size, because it runs on a background worker on
# somebody's laptop while they keep working. The two files must be the same
# export - a tokenizer from a different revision produces ids the graph was
# never trained on, and the result is not an error, it is quietly worse search.
if ($Model -eq "embedding") {
    $dir = Join-Path $PSScriptRoot "..\models\embedding"
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    $base = "https://huggingface.co/Xenova/bge-small-en-v1.5/resolve/main"
    Get-File "$base/onnx/model_quantized.onnx" (Join-Path $dir "model.onnx") 33
    Get-File "$base/tokenizer.json" (Join-Path $dir "tokenizer.json") 1
    Write-Host ""
    Write-Host "Embedding model ready. Check it with:" -ForegroundColor Green
    Write-Host "  cargo run -p memos-embed --features onnx --example embed_check"
    exit 0
}

# The Tier 1 router (ADR-0003). Qwen3 0.6B at Q8 rather than Q4: this model
# decodes under a GBNF grammar, so it is mostly choosing among tokens the
# grammar already allows, and what is left to get wrong is the intent decision
# itself - which is exactly what heavy quantisation costs you at this size. Q4
# is offered for the Light profile, at 378 MB.
if ($Model -eq "router" -or $Model -eq "router-light") {
    $dir = Join-Path $PSScriptRoot "..\models\llm"
    New-Item -ItemType Directory -Force -Path $dir | Out-Null

    if ($Model -eq "router") {
        $url = "https://huggingface.co/Qwen/Qwen3-0.6B-GGUF/resolve/main/Qwen3-0.6B-Q8_0.gguf"
        $mb = 609
    } else {
        $url = "https://huggingface.co/unsloth/Qwen3-0.6B-GGUF/resolve/main/Qwen3-0.6B-Q4_K_M.gguf"
        $mb = 378
    }
    # One filename whatever the quantisation, so the app finds the router
    # without being told which build the user chose.
    Get-File $url (Join-Path $dir "router.gguf") $mb
    Write-Host ""
    Write-Host "Router model ready. Check it with:" -ForegroundColor Green
    Write-Host "  cargo run -p memos-llm --example grammar_check"
    exit 0
}

$known = @{
    "tiny.en"   = 78
    "base.en"   = 148
    "small.en"  = 488
    "medium.en" = 1533
}
if (-not $known.ContainsKey($Model)) {
    Write-Host "Unknown model '$Model'. Choose one of: $($known.Keys -join ', '), embedding, router, router-light" -ForegroundColor Red
    exit 1
}

$dir = Join-Path $PSScriptRoot "..\models\stt"
New-Item -ItemType Directory -Force -Path $dir | Out-Null
Get-File "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-$Model.bin" `
         (Join-Path $dir "ggml-$Model.bin") $known[$Model]
