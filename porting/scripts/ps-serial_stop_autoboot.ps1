$portName = 'COM7'
$baud = 115200
$logFile = 'C:\Users\25418\Program\OS-Riscv\logs\autoboot_log.txt'

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

Log "Listening and sending 's' to stop autoboot..."
$start = Get-Date
$inUboot = $false
while (((Get-Date) - $start).TotalMilliseconds -lt 15000) {
    $out = Drain $port 500
    if ($out) {
        Log "RX: [$out]"
        if ($out -match '=>') { $inUboot = $true; break }
        if ($out -match 'Autoboot|Hit any key|Press .* to abort|login:|password:') {
            # send 's' to stop autoboot
            $port.Write(@([byte][char]'s'), 0, 1)
            Log "Sent 's'"
            Start-Sleep -Milliseconds 200
        }
    }
}

if ($inUboot) {
    Log "U-Boot prompt reached."
} else {
    Log "Did not reach U-Boot prompt in 15s"
}

$port.Close()
Log "Done"
