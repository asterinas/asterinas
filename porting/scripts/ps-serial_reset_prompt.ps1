$portName = 'COM7'
$baud = 115200
$logFile = 'C:\Users\25418\Program\OS-Riscv\logs\reset_prompt_log.txt'

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

Log "Draining serial buffer..."
$drained = Drain $port 3000
Log "Drained: [$drained]"

Log "Sending Ctrl-C and newlines to get a fresh prompt..."
for ($i = 0; $i -lt 3; $i++) {
    $port.Write(@([byte]0x03), 0, 1)
    Start-Sleep -Milliseconds 200
    $port.WriteLine("")
    Start-Sleep -Milliseconds 300
}

$start = Get-Date
$promptFound = $false
while (((Get-Date) - $start).TotalMilliseconds -lt 10000) {
    $out = Drain $port 1000
    if ($out -match '=>') {
        Log "Prompt found: [$out]"
        $promptFound = $true
        break
    }
}
if (-not $promptFound) {
    Log "Prompt not found within 10s"
}

$port.Close()
Log "Done"
