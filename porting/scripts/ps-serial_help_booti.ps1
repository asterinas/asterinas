$portName = 'COM7'
$baud = 115200
$port = New-Object System.IO.Ports.SerialPort($portName, $baud, 'None', 8, 'One')
$port.ReadTimeout = 1000
$port.WriteTimeout = 1000
$port.Open()

function Read-AllSerial {
    param($TimeoutMs)
    Start-Sleep -Milliseconds 200
    $start = Get-Date
    $buf = New-Object System.Text.StringBuilder
    while (((Get-Date) - $start).TotalMilliseconds -lt $TimeoutMs) {
        try {
            $line = $port.ReadExisting()
            if ($line) {
                [void]$buf.Append($line)
                $start = Get-Date
            }
        } catch {}
        Start-Sleep -Milliseconds 100
    }
    return $buf.ToString()
}

$null = Read-AllSerial -TimeoutMs 1000

$port.WriteLine("help booti")
Write-Host (Read-AllSerial -TimeoutMs 3000)

$port.WriteLine("help bootm")
Write-Host (Read-AllSerial -TimeoutMs 3000)

$port.Close()
