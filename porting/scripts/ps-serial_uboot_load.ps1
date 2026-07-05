param(
    [string]$port = "COM7",
    [int]$baud = 115200,
    [int]$timeoutSec = 300
)
$portObj = New-Object System.IO.Ports.SerialPort $port, $baud, "None", 8, "One"
$portObj.ReadTimeout = 2000
$portObj.Open()
Start-Sleep -Milliseconds 500
$null = $portObj.ReadExisting()
$sw = [Diagnostics.Stopwatch]::StartNew()
$prompt = $false
$output = ""
while ($sw.Elapsed.TotalSeconds -lt 60 -and -not $prompt) {
    try {
        $c = $portObj.ReadExisting()
        if ($c) {
            $output += $c
            [Console]::Write($c)
            if ($output -match '\=\>\s*$') { $prompt = $true }
        }
    } catch {}
    Start-Sleep -Milliseconds 100
}
if (-not $prompt) { Write-Error "U-Boot prompt not found"; exit 1 }
$portObj.WriteLine("ext4load mmc 1:3 0x80200000 /a/k")
Start-Sleep -Milliseconds 100
$loaded = $false
$output = ""
while ($sw.Elapsed.TotalSeconds -lt $timeoutSec -and -not $loaded) {
    try {
        $c = $portObj.ReadExisting()
        if ($c) {
            $output += $c
            [Console]::Write($c)
            if ($output -match '(\d+\s+bytes\s+read\s+in\s+\d+\s+ms|\=\>\s*$)') { $loaded = $true }
        }
    } catch {}
    Start-Sleep -Milliseconds 100
}
$portObj.Close()
