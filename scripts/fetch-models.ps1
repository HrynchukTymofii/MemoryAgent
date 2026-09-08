# Download the models the app needs.
#
#   .\scripts\fetch-models.ps1              # base.en  (~148 MB, the default)
#   .\scripts\fetch-models.ps1 tiny.en      # ~78 MB, for the Light profile
#   .\scripts\fetch-models.ps1 small.en     # ~488 MB, better accuracy
#   .\scripts\fetch-models.ps1 embedding    # ~33 MB, semantic search
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

$known = @{
    "tiny.en"   = 78
    "base.en"   = 148
    "small.en"  = 488
    "medium.en" = 1533
}
if (-not $known.ContainsKey($Model)) {
    Write-Host "Unknown model '$Model'. Choose one of: $($known.Keys -join ', '), embedding" -ForegroundColor Red
    exit 1
}

$dir = Join-Path $PSScriptRoot "..\models\stt"
New-Item -ItemType Directory -Force -Path $dir | Out-Null
Get-File "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-$Model.bin" `
         (Join-Path $dir "ggml-$Model.bin") $known[$Model]
