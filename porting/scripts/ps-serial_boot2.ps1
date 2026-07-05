$portName = 'COM7'
$baud = 115200
$logFile = 'C:\Users\25418\Program\OS-Riscv\logs\boot_log2.txt'

function Log($msg) {
    $line = "$(Get-Date -Format 'yyyy-MM-dd HH:mm:ss') $msg"
    Write-Host $line
    Add-Content -Path $logFile -Value $line -ErrorAction SilentlyContinue
}

Remove-Item -Path $logFile -ErrorAction SilentlyContinue
Log "Opening $portName"
$port = New-Object System.IO.Ports.SerialPort($portName, $baud, 'None', 8, 'One')
$port.ReadTimeout = 1000
$port.WriteTimeout = 1000
$port.Open()
Log "Port opened"

function Drain($port, $ms) {
    $port.ReadTimeout = 100
    $s = New-Object System.Text.StringBuilder
    $start = Get-Date
    while (((Get-Date) - $start).TotalMilliseconds -lt $ms) {
        try {
            $c = $port.ReadExisting()
            if ($c) { [void]$s.Append($c); $start = Get-Date }
        } catch {}
        Start-Sleep -Milliseconds 50
    }
    return $s.ToString()
}

$null = Drain $port 800

# Use actual DTB address from bdinfo
$cmd = 'bootm 0x80200000 - 0xed508e10'
Log "Sending: $cmd"
$port.WriteLine($cmd)

Log "Capturing output for 30 seconds..."
$start = Get-Date
$all = New-Object System.Text.StringBuilder
while (((Get-Date) - $start).TotalMilliseconds -lt 30000) {
    try {
        $c = $port.ReadExisting()
        if ($c) {
            [void]$all.Append($c)
            $start = Get-Date
        }
    } catch {}
    Start-Sleep -Milliseconds 100
}

$output = $all.ToString()
Log "Output:"
Write-Host $output
Add-Content -Path $logFile -Value $output

$port.Close()
Log "Done"
