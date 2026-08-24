# Verify a packaged Windows artifact carries a valid Authenticode signature.
# Usage: pwsh scripts/verify-windows-signature.ps1 [path]
#
# Environment:
#   BEARCAD_REQUIRE_SIGN=1
#       Fail when the file is unsigned or its signature does not validate.
#       Otherwise an unsigned file is reported and tolerated (local builds).
param([string]$Path = "bearcad.exe")

$ErrorActionPreference = "Stop"

if (-not (Test-Path -LiteralPath $Path)) {
    throw "Artifact not found: $Path"
}

$required = $env:BEARCAD_REQUIRE_SIGN -eq "1"
$sig = Get-AuthenticodeSignature -LiteralPath $Path

if ($sig.Status -eq "Valid") {
    Write-Host "$Path is signed by $($sig.SignerCertificate.Subject)"
    if ($sig.TimeStamperCertificate) {
        Write-Host "  timestamped by $($sig.TimeStamperCertificate.Subject)"
    } elseif ($required) {
        throw "$Path is signed but not timestamped; the signature expires with the certificate."
    } else {
        Write-Warning "$Path is signed but not timestamped."
    }
    exit 0
}

$message = "$Path signature status: $($sig.Status) - $($sig.StatusMessage)"
if ($required) {
    throw "BEARCAD_REQUIRE_SIGN=1 but $message"
}
Write-Warning $message
