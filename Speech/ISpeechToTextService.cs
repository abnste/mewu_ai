namespace mewu_ai_Assistant.Speech;
public interface ISpeechToTextService { Task<string?> RecognizeOnceAsync(string language,CancellationToken cancellationToken); }
