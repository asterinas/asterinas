param(
    [string]$port = "COM7",
    [int]$baud = 115200,
    [int]$timeoutSec = 30
)
$portObj = New-Object System.IO.Ports.SerialPort $port, $baud, "None", 8, "One"
$portObj.ReadTimeout = 2000
$portObj.Open()
Start-Sleep -Milliseconds 500
$null = $portObj.ReadExisting()
# Send Ctrl+C and newlines
$portObj.Write("`x03")
Start-Sleep -Milliseconds 200
for ($i=0; $i -lt 10; $i++) {
    $portObj.WriteLine("")
    Start-Sleep -Milliseconds 200
}
$sw = [Diagnostics.Stopwatch]::StartNew()
while ($sw.Elapsed.TotalSeconds -lt $timeoutSec) {
    try {
        $c = $portObj.ReadExisting()
        if ($c) { [Console]::Write($c) }
    } catch {}
    Start-Sleep -Milliseconds 100
}
$portObj.Close()
