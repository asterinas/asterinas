$portName = 'COM7'
$baud = 115200
$port = New-Object System.IO.Ports.SerialPort($portName, $baud, 'None', 8, 'One')
$port.ReadTimeout = 1000
$port.WriteTimeout = 1000
$port.Open()
Write-Host "Sending reset command"
$port.WriteLine('reset')
Start-Sleep -Milliseconds 500
$port.ReadTimeout = 100
$s = New-Object System.Text.StringBuilder
$start = Get-Date
while (((Get-Date) - $start).TotalMilliseconds -lt 3000) {
    try { $c = $port.ReadExisting(); if ($c) { [void]$s.Append($c); $start = Get-Date } } catch {}
    Start-Sleep -Milliseconds 50
}
Write-Host "Output: [$($s.ToString())]"
$port.Close()
