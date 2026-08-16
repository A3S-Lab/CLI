param(
    [Parameter(Mandatory = $true)]
    [string]$Destination
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
Set-StrictMode -Version Latest

$revision = '751bff37182d3f1213fa05d7196b954e230abad9'
$repository = 'Xenova/all-MiniLM-L6-v2'
$artifacts = @(
    @{
        Source = 'onnx/model_quantized.onnx'
        Name = 'model_quantized.onnx'
        Sha256 = 'afdb6f1a0e45b715d0bb9b11772f032c399babd23bfc31fed1c170afc848bdb1'
    },
    @{
        Source = 'tokenizer.json'
        Name = 'tokenizer.json'
        Sha256 = 'da0e79933b9ed51798a3ae27893d3c5fa4a201126cef75586296df9b4d2c62a0'
    },
    @{
        Source = 'config.json'
        Name = 'config.json'
        Sha256 = '7135149f7cffa1a573466c6e4d8423ed73b62fd2332c575bf738a0d033f70df7'
    },
    @{
        Source = 'special_tokens_map.json'
        Name = 'special_tokens_map.json'
        Sha256 = 'b6d346be366a7d1d48332dbc9fdf3bf8960b5d879522b7799ddba59e76237ee3'
    },
    @{
        Source = 'tokenizer_config.json'
        Name = 'tokenizer_config.json'
        Sha256 = '9261e7d79b44c8195c1cada2b453e55b00aeb81e907a6664974b4d7776172ab3'
    }
)

$destinationPath = [System.IO.Path]::GetFullPath($Destination)
New-Item -ItemType Directory -Force -Path $destinationPath | Out-Null

foreach ($artifact in $artifacts) {
    $target = Join-Path $destinationPath $artifact.Name
    $actual = if (Test-Path -LiteralPath $target -PathType Leaf) {
        (Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash.ToLowerInvariant()
    } else {
        $null
    }
    if ($actual -ne $artifact.Sha256) {
        $uri = "https://huggingface.co/$repository/resolve/$revision/$($artifact.Source)?download=true"
        for ($attempt = 1; $attempt -le 3; $attempt++) {
            try {
                Invoke-WebRequest -Uri $uri -OutFile $target -MaximumRedirection 5
                break
            } catch {
                if ($attempt -eq 3) {
                    throw
                }
                Start-Sleep -Seconds 2
            }
        }
        $actual = (Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    if ($actual -ne $artifact.Sha256) {
        throw "Local CPU embedding fixture digest mismatch for $($artifact.Name)"
    }
}

$fixtureManifest = [System.IO.Path]::GetFullPath(
    (Join-Path $PSScriptRoot '..\fixtures\local-cpu-embedding\model.acl')
)
$manifestPath = Join-Path $destinationPath 'model.acl'
Copy-Item -LiteralPath $fixtureManifest -Destination $manifestPath -Force
Write-Output $manifestPath
