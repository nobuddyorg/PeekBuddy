# Caps the memory of every process this PowerShell starts from now on.
#
# A mutant can turn a loop into one that allocates for ever; the runner then
# dies ("received a shutdown signal") and takes the whole shard with it. With
# the cap the allocation of the runaway process fails, its test fails and the
# mutant counts as caught.
#
#   . scripts\ci\memory-cap.ps1
#   Set-MemoryCap -Gigabytes 6
#
# The limit is per process (a rustc of the build needs less), set through a
# job object that this process and its children belong to.

function Set-MemoryCap {
    param([Parameter(Mandatory)][int]$Gigabytes)

    Add-Type -Namespace PeekBuddy -Name Job -MemberDefinition @'
[DllImport("kernel32.dll", SetLastError = true)]
public static extern System.IntPtr CreateJobObjectW(System.IntPtr attributes, string name);
[DllImport("kernel32.dll", SetLastError = true)]
public static extern bool SetInformationJobObject(System.IntPtr job, int infoClass, System.IntPtr info, int length);
[DllImport("kernel32.dll", SetLastError = true)]
public static extern bool AssignProcessToJobObject(System.IntPtr job, System.IntPtr process);
'@

    # JOBOBJECT_EXTENDED_LIMIT_INFORMATION on x64: 144 bytes, LimitFlags at 16,
    # ProcessMemoryLimit at 112.
    $size = 144
    $info = [System.Runtime.InteropServices.Marshal]::AllocHGlobal($size)
    try {
        for ($i = 0; $i -lt $size; $i += 8) {
            [System.Runtime.InteropServices.Marshal]::WriteInt64($info, $i, 0)
        }
        $JOB_OBJECT_LIMIT_PROCESS_MEMORY = 0x100
        [System.Runtime.InteropServices.Marshal]::WriteInt32($info, 16, $JOB_OBJECT_LIMIT_PROCESS_MEMORY)
        [System.Runtime.InteropServices.Marshal]::WriteInt64($info, 112, [int64]$Gigabytes * 1GB)

        $job = [PeekBuddy.Job]::CreateJobObjectW([System.IntPtr]::Zero, $null)
        if ($job -eq [System.IntPtr]::Zero) { throw "CreateJobObject failed: $([System.Runtime.InteropServices.Marshal]::GetLastWin32Error())" }
        $ExtendedLimitInformation = 9
        if (-not [PeekBuddy.Job]::SetInformationJobObject($job, $ExtendedLimitInformation, $info, $size)) {
            throw "SetInformationJobObject failed: $([System.Runtime.InteropServices.Marshal]::GetLastWin32Error())"
        }
        $self = (Get-Process -Id $PID).Handle
        if (-not [PeekBuddy.Job]::AssignProcessToJobObject($job, $self)) {
            throw "AssignProcessToJobObject failed: $([System.Runtime.InteropServices.Marshal]::GetLastWin32Error())"
        }
    }
    finally {
        [System.Runtime.InteropServices.Marshal]::FreeHGlobal($info)
    }
    Write-Host "memory cap: $Gigabytes GiB per process"
}
