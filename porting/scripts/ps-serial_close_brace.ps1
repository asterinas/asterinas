$portName = 'COM7'
$baud = 115200
$logFile = 'C:\Users\25418\Program\OS-Riscv\logs\brace_log.txt'

function Log($msg) {
    $line = "$(Get-Date -Format 'yyyy-MM-dd HH:mm:ss') $msg"
    Write-Host $line
    Add-Content -Path $logFile -Value $line -ErrorAction SilentlyContinue
}

Remove-Item -Path $logFile -ErrorAction SilentlyContinue
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

$null = Drain $port 1000
Log "Sending '}' to close unclosed variable reference"
$port.WriteLine('}')
Start-Sleep -Milliseconds 500
$out = Drain $port 3000
Log "Output after brace: [$out]"

Log "Sending 'version' command"
$port.WriteLine('version')
$out2 = Drain $port 3000
Log "Output after version: [$out2]"

$port.Close()
Log "Done"
