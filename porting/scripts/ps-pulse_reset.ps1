$p = New-Object System.IO.Ports.SerialPort 'COM7',115200,'None',8,'One'
$p.Open()
Start-Sleep -Milliseconds 200
$p.BreakState = $true
Start-Sleep -Milliseconds 500
$p.BreakState = $false
Start-Sleep -Milliseconds 200
$p.Close()
Write-Host 'BREAK sent'
