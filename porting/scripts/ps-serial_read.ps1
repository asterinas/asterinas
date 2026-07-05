param(
    [string]$port = "COM7",
    [int]$baud = 115200,
    [int]$timeoutSec = 10
)
$portObj = New-Object System.IO.Ports.SerialPort $port, $baud, "None", 8, "One"
$portObj.ReadTimeout = 2000
$portObj.Open()
Start-Sleep -Milliseconds 500
$null = $portObj.ReadExisting()
$sw = [Diagnostics.Stopwatch]::StartNew()
while ($sw.Elapsed.TotalSeconds -lt $timeoutSec) {
    try {
        $c = $portObj.ReadExisting()
        if ($c) { [Console]::Write($c) }
    } catch {}
    Start-Sleep -Milliseconds 100
}
$portObj.Close()
