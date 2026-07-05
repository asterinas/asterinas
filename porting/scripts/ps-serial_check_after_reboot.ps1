$portName = 'COM7'
$baud = 115200
$logFile = 'C:\Users\25418\Program\OS-Riscv\logs\reboot_check_log.txt'

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

Log "Draining 5s..."
$out = Drain $port 5000
Log "Drained: [$out]"

Log "Sending newline and Ctrl-C..."
$port.WriteLine("")
Start-Sleep -Milliseconds 200
$port.Write(@([byte]0x03),0,1)
Start-Sleep -Milliseconds 200
$out2 = Drain $port 5000
Log "After input: [$out2]"

$port.Close()
Log "Done"
