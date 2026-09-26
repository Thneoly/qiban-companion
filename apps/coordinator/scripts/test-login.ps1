Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/login.ps1"
$realRequest = (Get-Command Invoke-LoginRequest).ScriptBlock

function Assert-True($Value, [string]$Message) { if (!$Value) { throw $Message } }
function Read-Host([string]$Prompt, [switch]$AsSecureString) {
    $value = $script:inputs.Dequeue()
    if ($AsSecureString) { return ConvertTo-SecureString $value -AsPlainText -Force }
    return $value
}
function Invoke-LoginRequest([int]$ServicePort, [string]$Path, $Body, [string]$Token) {
    $script:calls.Add([pscustomobject]@{ Path = $Path; Body = ($Body | ConvertTo-Json -Compress); Token = $Token })
    return $script:responses.Dequeue()
}
function Reply([int]$Status, $Value) { return [pscustomobject]@{ Ok = ($Status -eq 200); Status = $Status; Value = $Value } }
function Prepare($Replies, $Inputs) {
    $script:responses = [Collections.Generic.Queue[object]]::new()
    foreach ($item in $Replies) { $script:responses.Enqueue($item) }
    $script:inputs = [Collections.Generic.Queue[string]]::new()
    foreach ($item in $Inputs) { $script:inputs.Enqueue($item) }
    $script:calls = [Collections.Generic.List[object]]::new()
}
$challenge = [guid]::NewGuid().ToString()
$profile = @{ accountId = [guid]::NewGuid().ToString(); companionId = [guid]::NewGuid().ToString() }
$token = 'qbs_' + ('a' * 43)
$health = Reply 200 @{ status = 'ok' }
$receipt = Reply 200 @{ challengeId = $challenge }
$verified = Reply 200 @{ accessToken = $token }

Prepare @($health, $receipt, (Reply 0 $null), $verified, (Reply 200 $profile)) @('a@example.com', '12345678')
$result = Start-CoordinatorLogin 4318
Assert-True ($result.accountId -eq $profile.accountId -and $result.companionId -eq $profile.companionId) 'Wrong identity output.'
Assert-True (@($result.PSObject.Properties).Count -eq 2) 'Unexpected output fields (possibly credentials).'
Assert-True (@($calls | Where-Object Path -eq '/v1/auth/request-code').Count -eq 1) 'Email request was retried.'
$verifyCalls = @($calls | Where-Object Path -eq '/v1/auth/verify-code')
Assert-True ($verifyCalls.Count -eq 2 -and $verifyCalls[0].Body -ceq $verifyCalls[1].Body) 'Lost response did not reuse the same login payload.'
$body = $verifyCalls[0].Body | ConvertFrom-Json
Assert-True ($body.nonce -cmatch '^[A-Za-z0-9_-]{43}$' -and $body.challengeId -eq $challenge) 'Invalid nonce/challenge.'
Assert-True ($calls[4].Token -ceq $token) 'Profile lookup lacked authentication.'
Write-Host 'PASS: lost verify response recovery, one email request, IDs-only result'

Prepare @($health, $receipt, (Reply 401 $null), $verified, (Reply 503 $null), (Reply 200 $profile)) @('a@example.com', '11111111', '22222222')
$null = Start-CoordinatorLogin 4318
$verifyCalls = @($calls | Where-Object Path -eq '/v1/auth/verify-code')
$first = $verifyCalls[0].Body | ConvertFrom-Json
$second = $verifyCalls[1].Body | ConvertFrom-Json
Assert-True ($first.nonce -ceq $second.nonce -and $first.code -cne $second.code) 'Retrying an incorrect code lost the challenge nonce.'
Assert-True (@($calls | Where-Object Path -eq '/v1/me').Count -eq 2) 'Profile temporary failure was not retried.'
Write-Host 'PASS: incorrect code correction and profile retry'

foreach ($status in @(0, 429, 503)) {
    Prepare @($health, (Reply $status $null)) @('a@example.com')
    $failed = $false
    try { $null = Start-CoordinatorLogin 4318 } catch { $failed = $true }
    Assert-True ($failed -and $calls.Count -eq 2) 'Failed email request retried or continued to verification.'
}
Prepare @($health, $receipt) @('a@example.com', 'q')
$result = Start-CoordinatorLogin 4318
Assert-True ($null -eq $result -and $calls.Count -eq 2) 'Cancel submitted a code.'
Prepare @($health, $receipt, (Reply 401 $null), (Reply 401 $null), (Reply 401 $null), (Reply 401 $null), (Reply 401 $null)) @('a@example.com', '11111111', '11111111', '11111111', '11111111', '11111111')
$failed = $false
try { $null = Start-CoordinatorLogin 4318 } catch { $failed = $true }
Assert-True ($failed -and @($calls | Where-Object Path -eq '/v1/auth/verify-code').Count -eq 5) 'Expired/exhausted code attempts were unbounded.'
Write-Host 'PASS: send failures never resend; cancellation and exhausted/expired code terminate'

function Invoke-RestMethod {
    param($Uri, $Method, $TimeoutSec, $MaximumRedirection, $ErrorAction, $ContentType, $Body, $Headers)
    $script:httpArguments = @{ Uri = $Uri; Redirects = $MaximumRedirection; Timeout = $TimeoutSec }
    throw 'secret-canary-do-not-print'
}
$response = & $realRequest 4321 '/v1/me' $null $token
Assert-True ($httpArguments.Uri -eq 'http://127.0.0.1:4321/v1/me' -and $httpArguments.Redirects -eq 0 -and $httpArguments.Timeout -eq 15) 'Unsafe HTTP target or redirects.'
Assert-True (!$response.Ok -and $response.Status -eq 0 -and $null -eq $response.Value) 'Raw transport exception leaked.'
$nonceA = New-LoginNonce
$nonceB = New-LoginNonce
Assert-True ($nonceA -cne $nonceB -and $nonceA.Length -eq 43) 'Separate login requests reused a nonce.'
Write-Host 'PASS: loopback-only requests, redirects disabled, transport error sanitized, fresh nonce'
$portFile = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot ('../../../.cache/login-port-' + [guid]::NewGuid().ToString('N') + '.clixml')))
try {
    Assert-True ((Get-LoginPort $portFile) -eq 4318) 'Default port incorrect.'
    [pscustomobject]@{ Port = 4321 } | Export-Clixml -LiteralPath $portFile
    Assert-True ((Get-LoginPort $portFile) -eq 4321) 'Saved launcher port was not read.'
    [pscustomobject]@{ Port = 0 } | Export-Clixml -LiteralPath $portFile
    $failed = $false
    try { $null = Get-LoginPort $portFile } catch { $failed = $true }
    Assert-True $failed 'Invalid saved port accepted.'
} finally { if (Test-Path -LiteralPath $portFile) { Remove-Item -LiteralPath $portFile } }
Write-Host 'PASS: actual CLIXML custom port and invalid-port rejection'
Write-Host 'Login workflow checks passed with mocked HTTP and input. No email sent.'
