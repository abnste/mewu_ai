using Windows.Globalization;
using Windows.Media.SpeechRecognition;
namespace mewu_ai_Assistant.Speech;
public sealed class WindowsSpeechToTextService : ISpeechToTextService
{
    public async Task<string?> RecognizeOnceAsync(string language,CancellationToken token)
    {
        using var recognizer=language=="system"?new SpeechRecognizer():new SpeechRecognizer(new Language(language));var compilation=await recognizer.CompileConstraintsAsync().AsTask(token);if(compilation.Status!=SpeechRecognitionResultStatus.Success)throw new InvalidOperationException("Windows 语音识别初始化失败");var result=await recognizer.RecognizeAsync().AsTask(token);return result.Status==SpeechRecognitionResultStatus.Success?result.Text:null;
    }
}
