[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Target,

    [Parameter(Mandatory = $true)]
    [ValidateSet('serein', 'mstsc')]
    [string]$Client,

    [Parameter(Mandatory = $true)]
    [ValidateSet('idle', 'typing', 'scroll', 'move-window', 'video', 'background-sftp', 'hidden')]
    [string]$Scenario,

    [ValidateRange(10, 900)]
    [int]$DurationSeconds = 90,

    [ValidateRange(200, 5000)]
    [int]$IntervalMs = 500,

    [string]$InterfaceAlias = '',

    [string]$OutputDirectory = (Join-Path $env:LOCALAPPDATA 'Serein\benchmarks')
)

$ErrorActionPreference = 'Stop'

function Get-ProcessSample([string]$Name) {
    $items = @(Get-Process -Name $Name -ErrorAction SilentlyContinue)
    $cpu = ($items | Measure-Object -Property CPU -Sum).Sum
    $rss = ($items | Measure-Object -Property WorkingSet64 -Sum).Sum
    if ($null -eq $cpu) { $cpu = 0 }
    if ($null -eq $rss) { $rss = 0 }
    [pscustomobject]@{ CpuSeconds = [double]$cpu; RssBytes = [long]$rss; Count = $items.Count }
}

function Get-NetworkSample {
    try {
        if ($InterfaceAlias) {
            $stats = @(Get-NetAdapterStatistics -Name $InterfaceAlias -ErrorAction Stop)
        } else {
            $up = @(Get-NetAdapter | Where-Object Status -eq 'Up' | Select-Object -ExpandProperty Name)
            $stats = @($up | ForEach-Object { Get-NetAdapterStatistics -Name $_ -ErrorAction Stop })
        }
        $sent = ($stats | Measure-Object -Property SentBytes -Sum).Sum
        $received = ($stats | Measure-Object -Property ReceivedBytes -Sum).Sum
        if ($null -eq $sent) { $sent = 0 }
        if ($null -eq $received) { $received = 0 }
        return [pscustomobject]@{ SentBytes = [long]$sent; ReceivedBytes = [long]$received }
    } catch {
        return [pscustomobject]@{ SentBytes = 0L; ReceivedBytes = 0L }
    }
}

function Get-RetransmittedSegments {
    try {
        $value = (Get-NetTCPStatistics -ErrorAction Stop).SegmentsRetransmitted
        if ($null -ne $value) { return [long]$value }
    } catch {}
    return $null
}

function Get-Percentile([double[]]$Values, [double]$Part) {
    if ($Values.Count -eq 0) { return $null }
    $sorted = @($Values | Sort-Object)
    $index = [Math]::Max(0, [Math]::Min($sorted.Count - 1, [Math]::Ceiling($sorted.Count * $Part) - 1))
    return [Math]::Round($sorted[$index], 2)
}

New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$baseName = "$stamp-$Client-$Scenario"
$csvPath = Join-Path $OutputDirectory "$baseName.csv"
$summaryPath = Join-Path $OutputDirectory "$baseName.summary.json"

$ping = [System.Net.NetworkInformation.Ping]::new()
$watch = [System.Diagnostics.Stopwatch]::StartNew()
$rows = [System.Collections.Generic.List[object]]::new()
$networkStart = Get-NetworkSample
$retransStart = Get-RetransmittedSegments

Write-Host "Measuring $Client / ${Scenario}: $DurationSeconds seconds. Keep repeating the same action."
while ($watch.Elapsed.TotalSeconds -lt $DurationSeconds) {
    $iterationStarted = $watch.ElapsedMilliseconds
    $reply = $null
    try { $reply = $ping.Send($Target, [Math]::Max(200, $IntervalMs)) } catch {}

    $serein = Get-ProcessSample 'serein'
    $helper = Get-ProcessSample 'serein-rdp'
    $mstsc = Get-ProcessSample 'mstsc'
    $network = Get-NetworkSample
    $pingOk = $null -ne $reply -and $reply.Status -eq [System.Net.NetworkInformation.IPStatus]::Success
    $pingMs = $null
    $pingStatus = 'Error'
    if ($null -ne $reply) { $pingStatus = $reply.Status.ToString() }
    if ($pingOk) { $pingMs = [double]$reply.RoundtripTime }

    $rows.Add([pscustomobject]@{
        TimestampUtc = (Get-Date).ToUniversalTime().ToString('o')
        ElapsedMs = $watch.ElapsedMilliseconds
        Client = $Client
        Scenario = $Scenario
        PingMs = $pingMs
        PingStatus = $pingStatus
        SereinCpuSeconds = $serein.CpuSeconds
        SereinRssMiB = [Math]::Round($serein.RssBytes / 1MB, 2)
        HelperCpuSeconds = $helper.CpuSeconds
        HelperRssMiB = [Math]::Round($helper.RssBytes / 1MB, 2)
        MstscCpuSeconds = $mstsc.CpuSeconds
        MstscRssMiB = [Math]::Round($mstsc.RssBytes / 1MB, 2)
        SentBytes = $network.SentBytes
        ReceivedBytes = $network.ReceivedBytes
    })

    $spent = $watch.ElapsedMilliseconds - $iterationStarted
    $remaining = $IntervalMs - $spent
    if ($remaining -gt 0) { Start-Sleep -Milliseconds $remaining }
}

$networkEnd = Get-NetworkSample
$retransEnd = Get-RetransmittedSegments
$rows | Export-Csv -LiteralPath $csvPath -NoTypeInformation -Encoding UTF8

$goodPings = @($rows | Where-Object { $null -ne $_.PingMs } | ForEach-Object { [double]$_.PingMs })
$first = $rows[0]
$last = $rows[$rows.Count - 1]
$summary = [ordered]@{
    schema = 1
    recordedAtUtc = (Get-Date).ToUniversalTime().ToString('o')
    target = $Target
    client = $Client
    scenario = $Scenario
    durationSeconds = [Math]::Round($watch.Elapsed.TotalSeconds, 2)
    intervalMs = $IntervalMs
    interfaceAlias = $InterfaceAlias
    samples = $rows.Count
    pingSuccessful = $goodPings.Count
    pingLost = $rows.Count - $goodPings.Count
    pingP50Ms = Get-Percentile $goodPings 0.50
    pingP95Ms = Get-Percentile $goodPings 0.95
    sentMiB = [Math]::Round(($networkEnd.SentBytes - $networkStart.SentBytes) / 1MB, 3)
    receivedMiB = [Math]::Round(($networkEnd.ReceivedBytes - $networkStart.ReceivedBytes) / 1MB, 3)
    sereinCpuSeconds = [Math]::Round($last.SereinCpuSeconds - $first.SereinCpuSeconds, 3)
    helperCpuSeconds = [Math]::Round($last.HelperCpuSeconds - $first.HelperCpuSeconds, 3)
    mstscCpuSeconds = [Math]::Round($last.MstscCpuSeconds - $first.MstscCpuSeconds, 3)
    sereinPeakRssMiB = ($rows | Measure-Object -Property SereinRssMiB -Maximum).Maximum
    helperPeakRssMiB = ($rows | Measure-Object -Property HelperRssMiB -Maximum).Maximum
    mstscPeakRssMiB = ($rows | Measure-Object -Property MstscRssMiB -Maximum).Maximum
    retransmittedSegments = if ($null -ne $retransStart -and $null -ne $retransEnd) { $retransEnd - $retransStart } else { $null }
    csv = $csvPath
}
$summary | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $summaryPath -Encoding UTF8

Write-Host "Done: $summaryPath"
$summary | Format-List
