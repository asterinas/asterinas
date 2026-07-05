param(
    [string]$port = "COM7",
    [int]$baud = 115200,
    [string]$cmd = "",
    [int]$timeoutSec = 300,
    [int]$charDelayMs = 20
)
$portObj = New-Object System.IO.Ports.SerialPort $port, $baud, "None", 8, "One"
$portObj.ReadTimeout = 2000
$portObj.Open()
Start-Sleep -Milliseconds 500
$null = $portObj.ReadExisting()
foreach ($c in $cmd.ToCharArray()) {
    $portObj.Write($c)
    Start-Sleep -Milliseconds $charDelayMs
}
$portObj.WriteLine("")
Start-Sleep -Milliseconds 100
$sw = [Diagnostics.Stopwatch]::StartNew()
while ($sw.Elapsed.TotalSeconds -lt $timeoutSec) {
    try {
        $ch = $portObj.ReadExisting()
        if ($ch) { [Console]::Write($ch) }
    } catch {}
    Start-Sleep -Milliseconds 100
}
$portObj.Close()
