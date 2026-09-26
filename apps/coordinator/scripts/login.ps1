[CmdletBinding()]
param([ValidateRange(1, 65535)][int]$Port = 4318)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function New-LoginNonce {
    $bytes = New-Object byte[] 32
    $rng = [Security.Cryptography.RandomNumberGenerator]::Create()
    try {
        $rng.GetBytes($bytes)
        return [Convert]::ToBase64String($bytes).TrimEnd('=').Replace('+', '-').Replace('/', '_')
    } finally { $rng.Dispose(); [Array]::Clear($bytes, 0, $bytes.Length) }
}

function Get-LoginPort([string]$SettingsPath) {
    if (!(Test-Path -LiteralPath $SettingsPath)) { return 4318 }
    # Read just the non-secret port; no DPAPI decryption or SMTP password import.
    [xml]$xml = [IO.File]::ReadAllText([IO.Path]::GetFullPath($SettingsPath))
    $node = $xml.SelectSingleNode('/*[local-name()="Objs"]/*[local-name()="Obj"]/*[local-name()="MS" or local-name()="Props"]/*[@N="Port"]')
    $savedPort = 0
    if ($null -eq $node -or ![int]::TryParse($node.InnerText, [ref]$savedPort) -or $savedPort -lt 1 -or $savedPort -gt 65535) { throw 'Invalid saved port; provide -Port explicitly.' }
    return $savedPort
}

function Invoke-LoginRequest([int]$ServicePort, [string]$Path, $Body, [string]$Token) {
    # A local-only client: never follow redirects carrying codes or credentials.
    $request = @{
        Uri = "http://127.0.0.1:$ServicePort$Path"
        Method = 'Get'; TimeoutSec = 15; MaximumRedirection = 0; ErrorAction = 'Stop'
    }
    if ($null -ne $Body) {
        $request.Method = 'Post'
        $request.ContentType = 'application/json'
        $request.Body = $Body | ConvertTo-Json -Compress
    }
    if ($Token) { $request.Headers = @{ Authorization = "Bearer $Token" } }
    try {
        $value = Invoke-RestMethod @request
        return [pscustomobject]@{ Ok = $true; Status = 200; Value = $value }
    } catch {
        $status = 0
        $response = $_.Exception.PSObject.Properties['Response']
        if ($null -ne $response -and $null -ne $response.Value) {
            try { $status = [int]$response.Value.StatusCode } catch { $status = 0 }
        }
        # Never echo exception text or upstream bodies, which may contain secrets.
        return [pscustomobject]@{ Ok = $false; Status = $status; Value = $null }
    }
}

function Invoke-RecoverableLoginRequest([int]$ServicePort, [string]$Path, $Body, [string]$Token) {
    $result = Invoke-LoginRequest $ServicePort $Path $Body $Token
    if (!$result.Ok -and ($result.Status -eq 0 -or $result.Status -ge 500)) {
        Write-Host 'Temporary connection/server failure; retrying once with the same login request.'
        # The same challenge, code and nonce recover a lost verification response.
        $result = Invoke-LoginRequest $ServicePort $Path $Body $Token
    }
    return $result
}

function Stop-LoginRequest([string]$Stage, [int]$Status) {
    switch ($Status) {
        0 { throw "$Stage failed: service unreachable or response lost. Check npm run coordinator. No automatic email resend." }
        400 { throw "$Stage rejected: invalid input or client/server versions do not match." }
        401 { throw "$Stage rejected: session/code expired or invalid. Run npm run coordinator:login again when ready." }
        429 { throw "$Stage rate limited: wait before retrying. Email cooldown is 60 seconds; hourly limits also apply." }
        503 { throw "$Stage unavailable: check the service and SMTP settings. No automatic email resend." }
        default { throw "$Stage failed (HTTP $Status). No credentials were printed." }
    }
}

function Start-CoordinatorLogin([int]$ServicePort) {
    $session = $null
    $code = $null
    $body = $null
    try {
        $health = Invoke-LoginRequest $ServicePort '/healthz' $null ''
        if (!$health.Ok) { Stop-LoginRequest 'Health check' $health.Status }
        Write-Host 'Local login: this will request one verification email. Use an invited email; q cancels.'
        $email = (Read-Host 'Invited email').Trim()
        if ($email -eq 'q') { return }
        try { $address = [Net.Mail.MailAddress]::new($email) } catch { throw 'Invalid email address.' }
        if ($address.Address -cne $email -or $email -match '[^\x21-\x7e]') { throw 'Use a plain ASCII email address.' }
        # Sending is never automatically retried: a timeout may follow delivery.
        $receipt = Invoke-LoginRequest $ServicePort '/v1/auth/request-code' @{ email = $email } ''
        if (!$receipt.Ok) { Stop-LoginRequest 'Request code' $receipt.Status }
        $challenge = [guid]::Empty
        if (![guid]::TryParse([string]$receipt.Value.challengeId, [ref]$challenge)) { throw 'Invalid challenge response.' }
        $nonce = New-LoginNonce
        Write-Host 'Request accepted. Check your inbox/spam folder. Uninvited addresses receive no mail; codes expire after 10 minutes.'
        for ($attempt = 0; $attempt -lt 5; $attempt++) {
            $secretCode = Read-Host '8-digit code (hidden; q cancels)' -AsSecureString
            try { $code = [Net.NetworkCredential]::new('', $secretCode).Password } finally { $secretCode.Dispose() }
            if ($code -eq 'q') { return }
            if ($code -notmatch '^[0-9]{8}$') { Write-Host 'Enter exactly 8 digits.'; continue }
            $body = @{ challengeId = $receipt.Value.challengeId; code = $code; nonce = $nonce }
            $verification = Invoke-RecoverableLoginRequest $ServicePort '/v1/auth/verify-code' $body ''
            if (!$verification.Ok) {
                if ($verification.Status -eq 401) {
                    Write-Host 'Code incorrect, expired, superseded or exhausted. Try another code from this request, or q to exit.'
                    continue
                }
                Stop-LoginRequest 'Verify code' $verification.Status
            }
            $session = $verification.Value
            if ([string]$session.accessToken -cnotmatch '^qbs_[A-Za-z0-9_-]{43}$') { throw 'Invalid session response.' }
            $profile = Invoke-RecoverableLoginRequest $ServicePort '/v1/me' $null $session.accessToken
            if (!$profile.Ok) { Stop-LoginRequest 'Load companion' $profile.Status }
            $accountId = [guid]::Empty
            $companionId = [guid]::Empty
            if (![guid]::TryParse([string]$profile.Value.accountId, [ref]$accountId) -or
                ![guid]::TryParse([string]$profile.Value.companionId, [ref]$companionId)) { throw 'Invalid companion response.' }
            Write-Host 'Login verified. Only IDs are displayed; the token is not saved or printed.'
            return [pscustomobject]@{ accountId = $accountId.ToString(); companionId = $companionId.ToString() }
        }
        throw 'Login attempts ended. Request a new code later with npm run coordinator:login.'
    } finally { $session = $null; $code = $null; $body = $null }
}

if ($MyInvocation.InvocationName -ne '.') {
    try {
        # Reuse only the local port from launcher settings; never use SMTP credentials here.
        $settings = Join-Path $PSScriptRoot '../../../.cache/coordinator/settings.clixml'
        if (!$PSBoundParameters.ContainsKey('Port')) { $Port = Get-LoginPort $settings }
        Start-CoordinatorLogin $Port
    } catch { Write-Host "Login: $($_.Exception.Message)" -ForegroundColor Red; exit 1 }
}
