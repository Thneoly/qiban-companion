[CmdletBinding()]
param(
    [switch]$Configure,
    [switch]$Check,
    [string]$DataDirectory
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$workspace = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../..'))

function Protect-LocalDirectory([string]$Path) {
    [void][IO.Directory]::CreateDirectory($Path)
    if ((Get-Item -LiteralPath $Path).Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw 'Use a regular local directory, not a link.'
    }
    $acl = [IO.Directory]::GetAccessControl($Path, [Security.AccessControl.AccessControlSections]::Access)
    $owner = [Security.Principal.WindowsIdentity]::GetCurrent().User
    $acl.SetAccessRuleProtection($true, $false)
    foreach ($existing in @($acl.Access)) { [void]$acl.RemoveAccessRuleSpecific($existing) }
    foreach ($sid in @($owner, [Security.Principal.SecurityIdentifier]::new('S-1-5-18'))) {
        $rule = [Security.AccessControl.FileSystemAccessRule]::new($sid, 'FullControl', 'ContainerInherit,ObjectInherit', 'None', 'Allow')
        $acl.AddAccessRule($rule)
    }
    [IO.Directory]::SetAccessControl($Path, $acl)
}

function Assert-Configuration($Config) {
    if ($Config.Version -ne 1 -or $Config.Port -lt 1 -or $Config.Port -gt 65535) { throw 'Invalid configuration version or port.' }
    if ($Config.Tls -notin @('starttls', 'tls')) { throw 'SMTP TLS must be starttls or tls.' }
    if ([string]::IsNullOrWhiteSpace($Config.Host) -or $Config.Host -match '[\s/:]') { throw 'Enter an SMTP hostname without a URL or port.' }
    if ([string]::IsNullOrWhiteSpace($Config.Username)) { throw 'SMTP username is required.' }
    if ($Config.Password -isnot [Security.SecureString] -or $Config.Password.Length -eq 0) { throw 'SMTP password is required.' }
    try { $null = [Net.Mail.MailAddress]::new($Config.From) } catch { throw 'Invalid sender mailbox.' }
    $emails = @($Config.Emails -split ',')
    if ($emails.Count -lt 1 -or $emails.Count -gt 100) { throw 'Provide between 1 and 100 invited emails.' }
    foreach ($email in $emails) {
        $address = $email.Trim()
        try { $parsed = [Net.Mail.MailAddress]::new($address) } catch { throw 'Invalid invited email.' }
        if ($address -cne $parsed.Address -or $address -match '[^\x21-\x7e]' -or $address.Length -gt 254) { throw 'Use plain ASCII invited email addresses.' }
    }
}

function Save-Configuration($Config, [string]$Path) {
    Assert-Configuration $Config
    # Export-Clixml encrypts SecureString with Windows DPAPI for this user/device.
    $temporary = "$Path.new"
    try {
        $Config | Export-Clixml -LiteralPath $temporary -Encoding UTF8
        Move-Item -LiteralPath $temporary -Destination $Path -Force
    } finally {
        if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary }
    }
}

function Read-ConfigurationWizard {
    Write-Host 'First-time setup / replace mail settings. Nothing is sent during setup.'
    Write-Host 'The SMTP password is hidden and saved encrypted for this Windows user.'
    $config = [pscustomobject]@{
        Version = 1
        Host = (Read-Host 'SMTP hostname (e.g. smtp.example.com)').Trim()
        Username = (Read-Host 'SMTP username').Trim()
        Password = (Read-Host 'SMTP password / app password' -AsSecureString)
        From = (Read-Host 'Sender email').Trim()
        Emails = (Read-Host 'Invited emails (comma-separated)').Trim()
        Tls = (Read-Host 'TLS mode: starttls=587, tls=465 [starttls]').Trim().ToLowerInvariant()
        Port = 4318
    }
    if (!$config.Tls) { $config.Tls = 'starttls' }
    $port = (Read-Host 'Local service port [4318]').Trim()
    if ($port) { $config.Port = [int]$port }
    Assert-Configuration $config
    return $config
}

function Initialize-AuthSecret([string]$Path) {
    if (Test-Path -LiteralPath $Path) {
        try { $bytes = [Convert]::FromBase64String([IO.File]::ReadAllText($Path).Trim()) }
        catch { throw 'Cannot read auth secret. Restore it from backup; it will not be overwritten.' }
        try {
            if ($bytes.Length -ne 32 -or @($bytes | Where-Object { $_ -ne 0 }).Count -eq 0) { throw 'Invalid auth secret; it will not be overwritten.' }
        } finally { [Array]::Clear($bytes, 0, $bytes.Length) }
        return
    }
    $bytes = New-Object byte[] 32
    $rng = [Security.Cryptography.RandomNumberGenerator]::Create()
    try {
        $rng.GetBytes($bytes)
        $stream = [IO.File]::Open($Path, [IO.FileMode]::CreateNew)
        $writer = [IO.StreamWriter]::new($stream)
        try { $writer.Write([Convert]::ToBase64String($bytes)) } finally { $writer.Dispose() }
    } finally {
        $rng.Dispose()
        [Array]::Clear($bytes, 0, $bytes.Length)
    }
}

function Assert-FreePort([int]$Port) {
    $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, $Port)
    $listener.Server.ExclusiveAddressUse = $true
    try { $listener.Start() } catch { throw 'Local port is occupied. Stop the other service or run npm run coordinator:configure to change the port.' }
    finally { $listener.Stop() }
}

function Start-Coordinator {
    if ($env:OS -ne 'Windows_NT') { throw 'This launcher requires Windows. See the manual guide for other systems.' }
    if (!(Get-Command cargo -ErrorAction SilentlyContinue)) { throw 'Rust/Cargo is missing. Install the Rust MSVC toolchain and C++ build tools first.' }
    if (!$DataDirectory) { $DataDirectory = Join-Path $workspace '.cache/coordinator' }
    $DataDirectory = [IO.Path]::GetFullPath($DataDirectory)
    $cacheRoot = [IO.Path]::GetFullPath((Join-Path $workspace '.cache'))
    if (!$DataDirectory.StartsWith($cacheRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Launcher data must be in a dedicated subdirectory of the workspace .cache directory.'
    }
    $ancestor = $DataDirectory
    while ($ancestor -and $ancestor.Length -ge $cacheRoot.Length) {
        if ((Test-Path -LiteralPath $ancestor) -and ((Get-Item -LiteralPath $ancestor).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw 'Launcher data directories must not contain links.'
        }
        $ancestor = [IO.Path]::GetDirectoryName($ancestor)
    }
    $configPath = Join-Path $DataDirectory 'settings.clixml'
    $secretPath = Join-Path $DataDirectory 'auth-secret'
    $dbPath = Join-Path $DataDirectory 'accounts.db'
    if ($Check -and $Configure) { throw 'Use either -Check or -Configure.' }
    if ($Check -and !(Test-Path -LiteralPath $configPath)) { throw 'No saved settings. Run npm run coordinator for first-time setup.' }
    # This directory belongs only to the launcher; never point it at a desktop data directory.
    Protect-LocalDirectory $DataDirectory
    if ($Configure -or !(Test-Path -LiteralPath $configPath)) {
        $config = Read-ConfigurationWizard
        Save-Configuration $config $configPath
    } else {
        try { $config = Import-Clixml -LiteralPath $configPath } catch { throw 'Cannot decrypt saved settings. Use the original Windows user or run npm run coordinator:configure.' }
        Assert-Configuration $config
    }
    if ($Check -and !(Test-Path -LiteralPath $secretPath)) { throw 'Auth secret is missing. Run npm run coordinator to initialize it.' }
    Initialize-AuthSecret $secretPath
    if ($Configure) { Write-Host 'Settings saved. Run npm run coordinator to start.'; return }
    Assert-FreePort $config.Port
    if ($Check) { Write-Host 'Local configuration, secret, Cargo and port checks passed. SMTP delivery and compilation were not tested.'; return }

    Write-Host 'Building coordinator (first run may download Rust dependencies)...'
    $names = @('CARGO_HOME', 'CARGO_TARGET_DIR', 'QIBAN_AUTH_SECRET_FILE', 'QIBAN_ALLOWED_EMAILS', 'QIBAN_SMTP_HOST', 'QIBAN_SMTP_USERNAME', 'QIBAN_SMTP_PASSWORD', 'QIBAN_SMTP_FROM', 'QIBAN_SMTP_TLS', 'QIBAN_COORDINATOR_DB', 'QIBAN_COORDINATOR_PORT')
    $previous = @{}
    foreach ($name in $names) { $previous[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
    Push-Location $workspace
    try {
        if (!$env:CARGO_HOME -and (Test-Path -LiteralPath (Join-Path $workspace '.cache/cargo'))) { $env:CARGO_HOME = Join-Path $workspace '.cache/cargo' }
        $env:CARGO_TARGET_DIR = Join-Path $workspace 'target'
        & cargo build -p companion-coordinator --locked
        if ($LASTEXITCODE -ne 0) { throw 'Build failed. Check the Cargo output above and the Rust/C++ toolchain.' }
        Assert-FreePort $config.Port
        $env:QIBAN_AUTH_SECRET_FILE = $secretPath
        $env:QIBAN_COORDINATOR_DB = $dbPath
        $env:QIBAN_ALLOWED_EMAILS = $config.Emails
        $env:QIBAN_SMTP_HOST = $config.Host
        $env:QIBAN_SMTP_USERNAME = $config.Username
        $env:QIBAN_SMTP_PASSWORD = [Net.NetworkCredential]::new('', $config.Password).Password
        $env:QIBAN_SMTP_FROM = $config.From
        $env:QIBAN_SMTP_TLS = $config.Tls
        $env:QIBAN_COORDINATOR_PORT = [string]$config.Port
        Write-Host "Starting at http://127.0.0.1:$($config.Port) (API only). Press Ctrl+C to stop."
        Write-Host 'Startup does not send mail. Local data is retained for the next run.'
        & (Join-Path $env:CARGO_TARGET_DIR 'debug/companion-coordinator.exe')
        if ($LASTEXITCODE -ne 0) { throw 'Coordinator stopped with an error.' }
    } finally {
        foreach ($name in $names) { [Environment]::SetEnvironmentVariable($name, $previous[$name], 'Process') }
        Pop-Location
    }
}

# Dot-sourcing exposes the helpers to the launcher regression tests without running setup.
if ($MyInvocation.InvocationName -ne '.') {
    try { Start-Coordinator } catch { Write-Host "Coordinator: $($_.Exception.Message)" -ForegroundColor Red; exit 1 }
}
