namespace mewu_ai_Assistant.Speech;

public enum SpeechRecognitionFailureContext
{
    General,
    RecognizerInitialization,
    AudioInput,
    Recognition
}

public static class SpeechRecognitionFailureMapper
{
    public const int DeviceBusy = unchecked((int)0x80045006);
    public const int DeviceNotSupported = unchecked((int)0x80045007);
    public const int DeviceNotEnabled = unchecked((int)0x80045008);
    public const int NoAudioDriver = unchecked((int)0x80045009);
    public const int NoAudioData = unchecked((int)0x80045030);
    public const int InvalidAudioState = unchecked((int)0x8004503B);
    public const int GenericAudioError = unchecked((int)0x8004503C);
    public const int UnsupportedLanguage = unchecked((int)0x80045059);
    public const int AudioBufferUnderflow = unchecked((int)0x8004505B);
    public const int AudioStoppedUnexpectedly = unchecked((int)0x8004505C);
    public const int RecognitionTimeout = unchecked((int)0x80045060);
    public const int RecognizerNotFound = unchecked((int)0x80045077);
    public const int AudioDeviceNotFound = unchecked((int)0x80045078);

    public static string FromException(
        Exception exception,
        SpeechRecognitionFailureContext context = SpeechRecognitionFailureContext.General)
    {
        ArgumentNullException.ThrowIfNull(exception);
        exception = Unwrap(exception);

        if (exception is UnauthorizedAccessException)
            return "麦克风权限未开启，无法使用语音输入";

        if (exception is TypeLoadException or FileNotFoundException or PlatformNotSupportedException)
            return "当前 Windows 缺少语音识别组件";

        if (exception.HResult == DeviceBusy)
            return "麦克风正被其他应用占用，请稍后重试";

        if (exception.HResult is DeviceNotSupported or DeviceNotEnabled or NoAudioDriver or AudioDeviceNotFound)
            return "未检测到可用麦克风";

        if (exception.HResult is NoAudioData or RecognitionTimeout)
            return "没有听到语音，请重试";

        if (exception.HResult is InvalidAudioState or GenericAudioError or AudioBufferUnderflow or
            AudioStoppedUnexpectedly)
            return "没有收到清晰的麦克风声音";

        if (context == SpeechRecognitionFailureContext.AudioInput)
            return "未检测到可用麦克风";

        if (context == SpeechRecognitionFailureContext.RecognizerInitialization ||
            exception.HResult is RecognizerNotFound or UnsupportedLanguage)
            return "当前语言缺少可用的语音识别器";

        if (context == SpeechRecognitionFailureContext.Recognition)
            return "没有收到清晰的麦克风声音";

        return "Windows 语音识别暂时不可用";
    }

    private static Exception Unwrap(Exception exception)
    {
        while (exception is AggregateException { InnerExceptions.Count: 1 } aggregate)
            exception = aggregate.InnerExceptions[0];
        return exception;
    }
}
