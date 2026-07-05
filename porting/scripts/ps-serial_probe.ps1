$portName = 'COM7'
$baud = 115200
$port = New-Object System.IO.Ports.SerialPort($portName, $baud, 'None', 8, 'One')
$port.ReadTimeout = 2000
$port.WriteTimeout = 2000
try {
    $port.Open()
    Write-Host "Opened $portName"
    Start-Sleep -Milliseconds 500
    # Send a newline to see if there is a prompt
    $port.WriteLine("")
    Start-Sleep -Milliseconds 500
    try {
        $data = $port.ReadExisting()
        Write-Host "READ: [$data]"
    } catch {
        Write-Host "READ TIMEOUT"
    }
    $port.Close()
} catch {
    Write-Host "ERROR: $_"
}
