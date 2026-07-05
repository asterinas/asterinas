param(
    [string]$port = "COM7",
    [int]$baud = 115200,
    [int]$timeoutSec = 120
)
$portObj = New-Object System.IO.Ports.SerialPort $port, $baud, "None", 8, "One"
$portObj.ReadTimeout = 2000
$portObj.Open()
Start-Sleep -Milliseconds 500
$null = $portObj.ReadExisting()
$portObj.WriteLine("")
Start-Sleep -Milliseconds 300
$portObj.WriteLine("root")
Start-Sleep -Milliseconds 300
$portObj.WriteLine("milkv")
Start-Sleep -Milliseconds 500
$sw = [Diagnostics.Stopwatch]::StartNew()
$prompt = $false
$output = ""
while ($sw.Elapsed.TotalSeconds -lt $timeoutSec -and -not $prompt) {
    try {
        $c = $portObj.ReadExisting()
        if ($c) {
            $output += $c
            [Console]::Write($c)
            if ($output -match 'root@rockos-eswin:~#\s*$') { $prompt = $true }
        }
    } catch {}
    Start-Sleep -Milliseconds 100
}
if ($prompt) {
    Start-Sleep -Milliseconds 300
    $portObj.WriteLine("reboot")
    Start-Sleep -Milliseconds 100
}
$uboot = $false
while ($sw.Elapsed.TotalSeconds -lt $timeoutSec -and -not $uboot) {
    try {
        $c = $portObj.ReadExisting()
        if ($c) {
            $output += $c
            [Console]::Write($c)
            if ($output -match '\=\>\s*$') { $uboot = $true }
        }
    } catch {}
    Start-Sleep -Milliseconds 100
}
$portObj.Close()
