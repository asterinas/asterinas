param(
    [string]$port = "COM7",
    [int]$baud = 115200,
    [string]$cmd = "",
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
while ($sw.Elapsed.TotalSeconds -lt 60 -and -not $prompt) {
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
if (-not $prompt) { Write-Error "Login prompt not found"; exit 1 }
if ($cmd) {
    Start-Sleep -Milliseconds 300
    $portObj.WriteLine($cmd)
    Start-Sleep -Milliseconds 100
    $done = $false
    $output = ""
    while ($sw.Elapsed.TotalSeconds -lt $timeoutSec -and -not $done) {
        try {
            $c = $portObj.ReadExisting()
            if ($c) {
                $output += $c
                [Console]::Write($c)
                if ($output -match 'root@rockos-eswin:~#\s*$') { $done = $true }
            }
        } catch {}
        Start-Sleep -Milliseconds 100
    }
}
$portObj.Close()
