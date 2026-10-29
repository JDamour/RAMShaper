$os = Get-CimInstance Win32_OperatingSystem
$totalKB = $os.TotalVisibleMemorySize
$freePhysKB = $os.FreePhysicalMemory
$commitLimitKB = $os.TotalVirtualMemorySize
$freeCommitKB = $os.FreeVirtualMemory
"physical total {0:N1} GiB, available {1:N1} GiB" -f ($totalKB/1MB), ($freePhysKB/1MB)
"commit limit {0:N1} GiB, free commit {1:N1} GiB (charged {2:N1} GiB)" -f ($commitLimitKB/1MB), ($freeCommitKB/1MB), (($commitLimitKB-$freeCommitKB)/1MB)
""
"top 10 commit holders:"
Get-Process | Sort-Object PagedMemorySize64 -Descending |
  Select-Object -First 10 Name, Id, @{n='CommitMB';e={[math]::Round($_.PagedMemorySize64/1MB)}} |
  Format-Table -AutoSize | Out-String -Width 80
