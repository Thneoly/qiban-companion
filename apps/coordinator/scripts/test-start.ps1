param([switch]$Smoke)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/start.ps1"

function Assert-True($Value, [string]$Message) {
    if (!$Value) { throw $Message }
}
function Assert-Rejected([scriptblock]$Action, [string]$Message) {
    $rejected = $false
    try { & $Action } catch { $rejected = $true }
    Assert-True $rejected $Message
}

$testDirectory = Join-Path $workspace ('.cache/launcher-test-' + [Guid]::NewGuid().ToString('N'))
Protect-LocalDirectory $testDirectory
$configPath = Join-Path $testDirectory 'settings.clixml'
$secretPath = Join-Path $testDirectory 'auth-secret'
$config = [pscustomobject]@{
    Version = 1; Host = 'smtp.example.invalid'; Username = 'test'
    Password = (ConvertTo-SecureString 'fixture-password-not-real' -AsPlainText -Force)
    From = 'test@example.invalid'; Emails = 'one@example.invalid,two@example.invalid'
    Tls = 'starttls'; Port = 4318
}
try {
    Save-Configuration $config $configPath
    $loaded = Import-Clixml -LiteralPath $configPath
    Assert-Configuration $loaded
    Assert-True ([Net.NetworkCredential]::new('', $loaded.Password).Password -eq 'fixture-password-not-real') 'DPAPI round trip failed.'
    Assert-True (![IO.File]::ReadAllText($configPath).Contains('fixture-password-not-real')) 'Password was persisted in plaintext.'
    $acl = Get-Acl -LiteralPath $testDirectory
    Assert-True $acl.AreAccessRulesProtected 'Private directory inherited permissions.'
    Assert-True ($acl.Access.Count -eq 2) 'Unexpected private directory access entries.'
    Write-Host 'PASS: protected configuration and DPAPI round trip'

    Initialize-AuthSecret $secretPath
    $original = [IO.File]::ReadAllText($secretPath)
    Initialize-AuthSecret $secretPath
    Assert-True ($original -ceq [IO.File]::ReadAllText($secretPath)) 'Existing secret changed.'
    Assert-True ([Convert]::FromBase64String($original).Length -eq 32) 'Invalid generated secret size.'
    $config.Host = 'updated.example.invalid'
    Save-Configuration $config $configPath
    Assert-True ($original -ceq [IO.File]::ReadAllText($secretPath)) 'Reconfiguration rotated the secret.'
    Write-Host 'PASS: repeat setup preserves identity secret'

    $config.Tls = 'plain'
    Assert-Rejected { Save-Configuration $config $configPath } 'Plaintext SMTP was accepted.'
    Assert-True ((Import-Clixml -LiteralPath $configPath).Tls -eq 'starttls') 'Invalid settings replaced saved configuration.'
    [IO.File]::WriteAllText($secretPath, 'invalid-secret')
    Assert-Rejected { Initialize-AuthSecret $secretPath } 'Malformed secret was accepted.'
    Assert-True ([IO.File]::ReadAllText($secretPath) -eq 'invalid-secret') 'Malformed secret was overwritten.'
    [IO.File]::WriteAllText($secretPath, $original)
    Write-Host 'PASS: invalid configuration and secret fail without replacing saved data'

    Assert-True ((Get-DefaultSmtpTls 'SMTP.MX.CLOUDFLARE.NET.') -eq 'tls') 'Cloudflare TLS default is incorrect.'
    Assert-True ((Get-DefaultSmtpTls 'smtp.example.invalid') -eq 'starttls') 'Generic SMTP default changed.'
    $config.Host = 'smtp.mx.cloudflare.net'
    $config.Username = 'api_token'
    $config.Tls = 'starttls'
    Assert-Rejected { Save-Configuration $config $configPath } 'Cloudflare accepted unsupported STARTTLS.'
    $config.Tls = 'tls'
    Assert-Configuration $config
    $config.Username = 'API_TOKEN'
    Assert-Rejected { Assert-Configuration $config } 'Cloudflare accepted an invalid username.'
    Assert-True ((Import-Clixml -LiteralPath $configPath).Host -eq 'updated.example.invalid') 'Rejected Cloudflare settings overwrote saved configuration.'
    Write-Host 'PASS: Cloudflare implicit TLS and exact username requirements'

    $newPassword = ConvertTo-SecureString 'new-mailbox-fixture' -AsPlainText -Force
    foreach ($providerName in @('qq', '163')) {
        $mailbox = "tester@$providerName.com"
        $preset = New-MailboxConfiguration $providerName $mailbox $newPassword $loaded
        Assert-True ($preset.Host -eq "smtp.$providerName.com" -and $preset.Tls -eq 'tls') 'Incorrect mailbox transport.'
        Assert-True ($preset.From -eq $mailbox -and $preset.Username -eq $mailbox) 'Old sender or username leaked into new provider.'
        Assert-True ($preset.Emails -eq $loaded.Emails -and $preset.Port -eq $loaded.Port) 'Existing recipient or port changed.'
        Assert-True ([Net.NetworkCredential]::new('', $preset.Password).Password -eq 'new-mailbox-fixture') 'Old provider password was reused.'
        Save-Configuration $preset $configPath
        Assert-True ($original -ceq [IO.File]::ReadAllText($secretPath)) 'Provider switch changed auth secret.'
    }
    $fresh = New-MailboxConfiguration 'qq' 'tester@qq.com' $newPassword $null
    Assert-True ($fresh.Emails -eq 'tester@qq.com' -and $fresh.Port -eq 4318) 'First-time mailbox defaults incorrect.'
    Assert-Rejected { New-MailboxConfiguration 'qq' 'sender@example.com' $newPassword $loaded } 'Mismatched mailbox domain accepted.'
    Save-Configuration $loaded $configPath
    Write-Host 'PASS: QQ/163 provider replacement preserves recipients and secret without reusing old credentials'

    $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
    $listener.Server.ExclusiveAddressUse = $true
    $listener.Start()
    $port = $listener.LocalEndpoint.Port
    try { Assert-Rejected { Assert-FreePort $port } 'Occupied port was accepted.' }
    finally { $listener.Stop() }
    Assert-FreePort $port
    $loaded.Port = $port
    Save-Configuration $loaded $configPath
    Write-Host 'PASS: occupied port detection'

    & powershell -NoProfile -ExecutionPolicy Bypass -File "$PSScriptRoot/start.ps1" -Check -DataDirectory $testDirectory
    Assert-True ($LASTEXITCODE -eq 0) 'Saved-settings check failed.'
    & powershell -NoProfile -ExecutionPolicy Bypass -File "$PSScriptRoot/start.ps1" -Check -DataDirectory (Join-Path $testDirectory 'missing')
    Assert-True ($LASTEXITCODE -eq 1) 'Missing settings did not fail without prompting.'
    Assert-True (!(Test-Path -LiteralPath (Join-Path $testDirectory 'missing'))) 'Check created missing configuration.'
    Write-Host 'PASS: noninteractive checks and missing configuration'
    if ($Smoke) {
        # Warm the build first so the health-check budget below measures
        # startup only, never a cold cargo build or dependency downloads.
        Push-Location $workspace
        try {
            if (!$env:CARGO_HOME -and (Test-Path -LiteralPath (Join-Path $workspace '.cache/cargo'))) { $env:CARGO_HOME = Join-Path $workspace '.cache/cargo' }
            $env:CARGO_TARGET_DIR = Join-Path $workspace 'target'
            & cargo build -p companion-coordinator --locked | Out-Null
            if ($LASTEXITCODE -ne 0) { throw 'Warm-up build failed; fix compilation before the smoke run.' }
        } finally { Pop-Location }
        $process = Start-Process powershell -WindowStyle Hidden -PassThru -ArgumentList @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$PSScriptRoot/start.ps1`"", '-DataDirectory', "`"$testDirectory`"") -RedirectStandardOutput (Join-Path $testDirectory 'stdout.log') -RedirectStandardError (Join-Path $testDirectory 'stderr.log')
        try {
            $ready = $false
            $deadline = [DateTime]::UtcNow.AddSeconds(60)
            while ([DateTime]::UtcNow -lt $deadline -and !$process.HasExited) {
                try {
                    $health = Invoke-RestMethod "http://127.0.0.1:$port/healthz" -TimeoutSec 1
                    if ($health.status -eq 'ok') { $ready = $true; break }
                } catch { Start-Sleep -Milliseconds 250 }
                $process.Refresh()
            }
            Assert-True $ready 'Real coordinator did not become healthy within 60 seconds.'
            Assert-True (Test-Path -LiteralPath (Join-Path $testDirectory 'accounts.db')) 'Dedicated database was not created.'
            Assert-True ($original -ceq [IO.File]::ReadAllText($secretPath)) 'Startup changed the secret.'
            Write-Host 'PASS: actual launcher build/start and HTTP health (no SMTP requests)'
        } finally {
            # Stop only this test process and its direct coordinator child, never an existing service.
            Get-CimInstance Win32_Process -Filter "ParentProcessId = $($process.Id)" | Where-Object { $_.Name -eq 'companion-coordinator.exe' } | ForEach-Object { Stop-Process -Id $_.ProcessId -ErrorAction SilentlyContinue }
            if (!$process.WaitForExit(5000)) { Stop-Process -Id $process.Id -ErrorAction SilentlyContinue }
            $process.Dispose()
        }
    }
    Write-Host 'Launcher checks passed. No email sent.'
} finally {
    # Only exact files created by this test are removed; no recursive cleanup.
    $files = @($configPath, $secretPath) + @('accounts.db', 'accounts.db-shm', 'accounts.db-wal', 'stdout.log', 'stderr.log' | ForEach-Object { Join-Path $testDirectory $_ })
    foreach ($file in $files) {
        if (Test-Path -LiteralPath $file) { Remove-Item -LiteralPath $file }
    }
    [IO.Directory]::Delete($testDirectory)
}
