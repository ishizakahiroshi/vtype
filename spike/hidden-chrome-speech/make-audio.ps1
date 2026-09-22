# 使い捨て実験。日本語と英語の文を 16kHz モノラル 16bit WAV にする。
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Speech

$here = Split-Path -Parent $MyInvocation.MyCommand.Path

function Get-WavInfo([string] $Path) {
    $bytes = [System.IO.File]::ReadAllBytes($Path)
    if ($bytes.Length -lt 44) { throw "wav too small: $Path" }
    $ascii = [System.Text.Encoding]::ASCII
    if ($ascii.GetString($bytes, 0, 4) -ne 'RIFF' -or $ascii.GetString($bytes, 8, 4) -ne 'WAVE') {
        throw "not a wav: $Path"
    }
    $offset = 12
    $sampleRate = 0
    $channels = 0
    $bits = 0
    $dataBytes = 0
    while ($offset + 8 -le $bytes.Length) {
        $id = $ascii.GetString($bytes, $offset, 4)
        $size = [BitConverter]::ToInt32($bytes, $offset + 4)
        if ($size -lt 0) { throw "bad chunk size in $Path" }
        $body = $offset + 8
        if ($id -eq 'fmt ') {
            if ($body + 16 -gt $bytes.Length) { throw "fmt truncated: $Path" }
            $channels = [BitConverter]::ToInt16($bytes, $body + 2)
            $sampleRate = [BitConverter]::ToInt32($bytes, $body + 4)
            $bits = [BitConverter]::ToInt16($bytes, $body + 14)
        } elseif ($id -eq 'data') {
            $dataBytes = $size
            break
        }
        $step = $size + ($size % 2)
        $offset = $body + $step
    }
    if ($sampleRate -le 0 -or $channels -le 0 -or $bits -le 0 -or $dataBytes -le 0) {
        throw "wav header incomplete: $Path"
    }
    $seconds = $dataBytes / ($sampleRate * $channels * ($bits / 8.0))
    [pscustomobject]@{
        Path       = $Path
        SampleRate = $sampleRate
        Channels   = $channels
        Bits       = $bits
        DataBytes  = $dataBytes
        Seconds    = [math]::Round($seconds, 3)
    }
}

function Write-SpeechWav {
    param(
        [System.Speech.Synthesis.SpeechSynthesizer] $Synth,
        [string] $Text,
        [string] $Path,
        [string] $VoiceName,
        [System.Speech.AudioFormat.SpeechAudioFormatInfo] $Format,
        [int] $Rate
    )
    if (Test-Path -LiteralPath $Path) { Remove-Item -LiteralPath $Path -Force }
    $Synth.Rate = $Rate
    $Synth.Volume = 100
    if ($VoiceName) { $Synth.SelectVoice($VoiceName) }
    $Synth.SetOutputToWaveFile($Path, $Format)
    try {
        $Synth.Speak($Text)
    } finally {
        $Synth.SetOutputToNull()
    }
}

$format = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(
    16000,
    [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen,
    [System.Speech.AudioFormat.AudioChannel]::Mono
)

$synth = New-Object System.Speech.Synthesis.SpeechSynthesizer
try {
    $enabled = @($synth.GetInstalledVoices() | Where-Object { $_.Enabled })
    if ($enabled.Count -eq 0) { throw 'No enabled speech voice is installed.' }

    $ja = $enabled | Where-Object { $_.VoiceInfo.Name -eq 'Microsoft Haruka Desktop' } | Select-Object -First 1
    if (-not $ja) {
        $ja = $enabled | Where-Object { $_.VoiceInfo.Culture.Name -like 'ja*' } | Select-Object -First 1
    }
    $jaFallback = $false
    if (-not $ja) {
        $ja = $enabled | Select-Object -First 1
        $jaFallback = $true
    }

    $en = $enabled | Where-Object { $_.VoiceInfo.Name -eq 'Microsoft Zira Desktop' } | Select-Object -First 1
    if (-not $en) {
        $en = $enabled | Where-Object { $_.VoiceInfo.Culture.Name -like 'en*' } | Select-Object -First 1
    }
    if (-not $en) { $en = $enabled | Select-Object -First 1 }

    $jaPath = Join-Path $here 'ja.wav'
    $enPath = Join-Path $here 'en.wav'
    $jaText = '今日は良い天気です。音声入力のテストです。'
    $enText = 'The weather is fine today. This is a speech input test.'

    Write-SpeechWav -Synth $synth -Text $jaText -Path $jaPath -VoiceName $ja.VoiceInfo.Name -Format $format -Rate 0
    $jaInfo = Get-WavInfo $jaPath
    if ($jaInfo.Seconds -lt 1) {
        Write-SpeechWav -Synth $synth -Text ($jaText + $jaText) -Path $jaPath -VoiceName $ja.VoiceInfo.Name -Format $format -Rate -5
        $jaInfo = Get-WavInfo $jaPath
    }

    Write-SpeechWav -Synth $synth -Text $enText -Path $enPath -VoiceName $en.VoiceInfo.Name -Format $format -Rate 0
    $enInfo = Get-WavInfo $enPath
    if ($enInfo.Seconds -lt 1) {
        Write-SpeechWav -Synth $synth -Text ($enText + $enText) -Path $enPath -VoiceName $en.VoiceInfo.Name -Format $format -Rate -5
        $enInfo = Get-WavInfo $enPath
    }
} finally {
    $synth.Dispose()
}

function Export-CanonicalWav([string] $Path, $Info) {
    # System.Speech writes fmt chunk size 18. Some readers want a 16-byte PCM fmt.
    $bytes = [System.IO.File]::ReadAllBytes($Path)
    $ascii = [System.Text.Encoding]::ASCII
    $offset = 12
    $dataOffset = -1
    $dataBytes = 0
    while ($offset + 8 -le $bytes.Length) {
        $id = $ascii.GetString($bytes, $offset, 4)
        $size = [BitConverter]::ToInt32($bytes, $offset + 4)
        if ($id -eq 'data') {
            $dataOffset = $offset + 8
            $dataBytes = $size
            break
        }
        $offset = $offset + 8 + $size + ($size % 2)
    }
    if ($dataOffset -lt 0) { throw "no data chunk: $Path" }
    $pcm = New-Object byte[] (44 + $dataBytes)
    [System.Buffer]::BlockCopy([System.Text.Encoding]::ASCII.GetBytes('RIFF'), 0, $pcm, 0, 4)
    [BitConverter]::GetBytes([int] (36 + $dataBytes)).CopyTo($pcm, 4)
    [System.Buffer]::BlockCopy([System.Text.Encoding]::ASCII.GetBytes('WAVE'), 0, $pcm, 8, 4)
    [System.Buffer]::BlockCopy([System.Text.Encoding]::ASCII.GetBytes('fmt '), 0, $pcm, 12, 4)
    [BitConverter]::GetBytes([int] 16).CopyTo($pcm, 16)
    [BitConverter]::GetBytes([int16] 1).CopyTo($pcm, 20)
    [BitConverter]::GetBytes([int16] $Info.Channels).CopyTo($pcm, 22)
    [BitConverter]::GetBytes([int] $Info.SampleRate).CopyTo($pcm, 24)
    $block = [int] ($Info.Channels * ($Info.Bits / 8))
    [BitConverter]::GetBytes([int] ($Info.SampleRate * $block)).CopyTo($pcm, 28)
    [BitConverter]::GetBytes([int16] $block).CopyTo($pcm, 32)
    [BitConverter]::GetBytes([int16] $Info.Bits).CopyTo($pcm, 34)
    [System.Buffer]::BlockCopy([System.Text.Encoding]::ASCII.GetBytes('data'), 0, $pcm, 36, 4)
    [BitConverter]::GetBytes([int] $dataBytes).CopyTo($pcm, 40)
    [System.Buffer]::BlockCopy($bytes, $dataOffset, $pcm, 44, $dataBytes)
    [System.IO.File]::WriteAllBytes($Path, $pcm)
}

Export-CanonicalWav $jaPath $jaInfo
Export-CanonicalWav $enPath $enInfo

function Add-WavSilence([string] $Path, [int] $Milliseconds) {
    # A little trailing silence lets a looped file produce isFinal instead of one endless interim.
    $info = Get-WavInfo $Path
    $extra = [int] ($info.SampleRate * $Milliseconds / 1000) * $info.Channels * ($info.Bits / 8)
    $bytes = [System.IO.File]::ReadAllBytes($Path)
    $grown = New-Object byte[] ($bytes.Length + $extra)
    [System.Buffer]::BlockCopy($bytes, 0, $grown, 0, $bytes.Length)
    $riffSize = [BitConverter]::ToInt32($grown, 4) + $extra
    [BitConverter]::GetBytes([int] $riffSize).CopyTo($grown, 4)
    $dataSize = [BitConverter]::ToInt32($grown, 40) + $extra
    [BitConverter]::GetBytes([int] $dataSize).CopyTo($grown, 40)
    [System.IO.File]::WriteAllBytes($Path, $grown)
}

Add-WavSilence $jaPath 800
Add-WavSilence $enPath 800
$jaInfo = Get-WavInfo $jaPath
$enInfo = Get-WavInfo $enPath

if ($jaInfo.Seconds -lt 1 -or $enInfo.Seconds -lt 1) {
    throw "WAV shorter than 1s. ja=$($jaInfo.Seconds) en=$($enInfo.Seconds)"
}
if ($jaInfo.SampleRate -ne 16000 -or $enInfo.SampleRate -ne 16000 -or $jaInfo.Channels -ne 1 -or $enInfo.Channels -ne 1 -or $jaInfo.Bits -ne 16 -or $enInfo.Bits -ne 16) {
    throw "WAV format is not 16kHz mono 16bit. ja=$($jaInfo.SampleRate)/$($jaInfo.Channels)/$($jaInfo.Bits) en=$($enInfo.SampleRate)/$($enInfo.Channels)/$($enInfo.Bits)"
}

Write-Output ("ja_voice=" + $ja.VoiceInfo.Name)
Write-Output ("ja_culture=" + $ja.VoiceInfo.Culture.Name)
Write-Output ("ja_fallback=" + $jaFallback)
Write-Output ("ja_seconds=" + $jaInfo.Seconds)
Write-Output ("ja_rate=" + $jaInfo.SampleRate)
Write-Output ("ja_channels=" + $jaInfo.Channels)
Write-Output ("ja_bits=" + $jaInfo.Bits)
Write-Output ("en_voice=" + $en.VoiceInfo.Name)
Write-Output ("en_culture=" + $en.VoiceInfo.Culture.Name)
Write-Output ("en_seconds=" + $enInfo.Seconds)
Write-Output ("en_rate=" + $enInfo.SampleRate)
Write-Output ("en_channels=" + $enInfo.Channels)
Write-Output ("en_bits=" + $enInfo.Bits)
