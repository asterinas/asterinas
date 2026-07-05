# PowerShell serial bridge for Milk-V Megrez COM7
# Reads COM7 continuously and appends to serial.log
# Sends any text written to serial_cmd.txt one character at a time to avoid
# overflowing U-Boot's serial input FIFO.
# Create serial.stop to exit cleanly.

$portName = "COM7"
$baud = 115200
$logFile = "C:\Users\25418\Program\serial.log"
$cmdFile = "C:\Users\25418\Program\serial_cmd.txt"
$stopFile = "C:\Users\25418\Program\serial.stop"

# Clear old log and any stale command/stop files
try { Remove-Item $logFile -ErrorAction SilentlyContinue } catch {}
try { Remove-Item $cmdFile -ErrorAction SilentlyContinue } catch {}
try { Remove-Item $stopFile -ErrorAction SilentlyContinue } catch {}

function OpenPort {
    $p = New-Object System.IO.Ports.SerialPort $portName, $baud, "None", 8, "One"
    $p.ReadTimeout = 100
    $p.WriteTimeout = 1000
    # DTR/RTS often reset the board; disable them.
    $p.DtrEnable = $false
    $p.RtsEnable = $false
    $p.Open()
    return $p
}

function Send-Slow($port, $text) {
    foreach ($ch in $text.ToCharArray()) {
        try {
            $port.Write($ch, 0, 1)
            Start-Sleep -Milliseconds 2
        } catch {
            [System.IO.File]::AppendAllText($logFile, "`n[SEND ERROR: $($_.Exception.Message)]`n")
            return $false
        }
    }
    return $true
}

while (-not (Test-Path $stopFile)) {
    try {
        $port = OpenPort
        [System.IO.File]::AppendAllText($logFile, "[$(Get-Date -Format 'HH:mm:ss')] OPENED $portName`n")
    } catch {
        Start-Sleep -Seconds 1
        continue
    }

    while (-not (Test-Path $stopFile)) {
        try {
            $data = $port.ReadExisting()
            if ($data) {
                [System.IO.File]::AppendAllText($logFile, $data)
            }
        } catch {
            [System.IO.File]::AppendAllText($logFile, "`n[PORT ERROR: $($_.Exception.Message)]`n")
            break
        }

        if (Test-Path $cmdFile) {
            $cmd = Get-Content $cmdFile -Raw
            Remove-Item $cmdFile -ErrorAction SilentlyContinue
            if ($cmd) {
                if (-not $cmd.EndsWith("`r`n")) { $cmd += "`r`n" }
                [System.IO.File]::AppendAllText($logFile, "`n[SENT] $cmd")
                $ok = Send-Slow $port $cmd
                if ($ok) {
                    [System.IO.File]::AppendAllText($logFile, "[SEND OK]`n")
                }
            }
        }

        Start-Sleep -Milliseconds 20
    }

    try { $port.Close() } catch {}
    Start-Sleep -Seconds 1
}

try { Remove-Item $stopFile -ErrorAction SilentlyContinue } catch {}
