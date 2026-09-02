using System.Globalization;
using System.Speech.Recognition;

namespace mewu_ai_Assistant.Speech;

public sealed class WindowsSpeechToTextService
{
    private readonly Func<string, CancellationToken, Task<string?>> _recognizeCore;

    public WindowsSpeechToTextService() : this(RecognizeOnceCoreAsync)
    {
    }

    internal WindowsSpeechToTextService(Func<string, CancellationToken, Task<string?>> recognizeCore)
    {
        _recognizeCore = recognizeCore ?? throw new ArgumentNullException(nameof(recognizeCore));
    }

    public Task<string?> RecognizeOnceAsync(string language, CancellationToken cancellationToken) =>
        Task.Run(() => _recognizeCore(language, cancellationToken), cancellationToken);

    private static async Task<string?> RecognizeOnceCoreAsync(string language, CancellationToken cancellationToken)
    {
        cancellationToken.ThrowIfCancellationRequested();
        SpeechRecognitionEngine? recognizer = null;

        try
        {
            recognizer = CreateRecognizer(language);
            cancellationToken.ThrowIfCancellationRequested();
            recognizer.InitialSilenceTimeout = TimeSpan.FromSeconds(8);
            recognizer.BabbleTimeout = TimeSpan.FromSeconds(4);
            recognizer.EndSilenceTimeout = TimeSpan.FromMilliseconds(1200);
            recognizer.EndSilenceTimeoutAmbiguous = TimeSpan.FromSeconds(2);
            recognizer.LoadGrammar(new DictationGrammar());
            cancellationToken.ThrowIfCancellationRequested();

            try
            {
                recognizer.SetInputToDefaultAudioDevice();
            }
            catch (Exception ex) when (ex is not OperationCanceledException)
            {
                throw ToUnavailable(ex, SpeechRecognitionFailureContext.AudioInput);
            }

            cancellationToken.ThrowIfCancellationRequested();
            return await RecognizeSingleAsync(recognizer, cancellationToken);
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            throw;
        }
        catch (Exception) when (cancellationToken.IsCancellationRequested)
        {
            throw new OperationCanceledException(cancellationToken);
        }
        catch (SpeechRecognitionUnavailableException)
        {
            throw;
        }
        catch (Exception ex)
        {
            throw ToUnavailable(ex, SpeechRecognitionFailureContext.General);
        }
        finally
        {
            try
            {
                recognizer?.Dispose();
            }
            catch (Exception)
            {
                // Recognition has already ended; cleanup must not replace the actionable result.
            }
        }
    }

    private static SpeechRecognitionEngine CreateRecognizer(string language)
    {
        try
        {
            var installed = SpeechRecognitionEngine.InstalledRecognizers();
            var selectedCulture = SpeechRecognizerLanguageSelector.SelectBestCulture(
                language,
                installed.Select(item => item.Culture),
                CultureInfo.CurrentUICulture,
                CultureInfo.CurrentCulture);

            if (selectedCulture is null)
                throw new SpeechRecognitionUnavailableException("当前语言缺少可用的语音识别器");

            var selected = installed.First(item =>
                string.Equals(item.Culture.Name, selectedCulture.Name, StringComparison.OrdinalIgnoreCase));
            return new SpeechRecognitionEngine(selected.Id);
        }
        catch (SpeechRecognitionUnavailableException)
        {
            throw;
        }
        catch (Exception ex)
        {
            throw ToUnavailable(ex, SpeechRecognitionFailureContext.RecognizerInitialization);
        }
    }

    private static async Task<string?> RecognizeSingleAsync(
        SpeechRecognitionEngine recognizer,
        CancellationToken cancellationToken)
    {
        var completion = new TaskCompletionSource<string?>(TaskCreationOptions.RunContinuationsAsynchronously);
        var bestCandidate = string.Empty;
        var heardAudio = false;
        EventHandler<RecognizeCompletedEventArgs>? completed = null;
        EventHandler<SpeechHypothesizedEventArgs>? hypothesized = null;
        EventHandler<SpeechRecognitionRejectedEventArgs>? rejected = null;
        EventHandler<SpeechRecognizedEventArgs>? recognized = null;
        EventHandler<AudioLevelUpdatedEventArgs>? audioLevelUpdated = null;
        hypothesized = (_, args) =>
        {
            if (!string.IsNullOrWhiteSpace(args.Result?.Text)) bestCandidate = args.Result.Text.Trim();
        };
        rejected = (_, args) =>
        {
            if (!string.IsNullOrWhiteSpace(args.Result?.Text)) bestCandidate = args.Result.Text.Trim();
        };
        recognized = (_, args) =>
        {
            if (!string.IsNullOrWhiteSpace(args.Result?.Text))
                completion.TrySetResult(args.Result.Text.Trim());
        };
        audioLevelUpdated = (_, args) => heardAudio |= args.AudioLevel > 0;
        completed = (_, args) =>
        {
            if (cancellationToken.IsCancellationRequested || args.Cancelled)
            {
                completion.TrySetCanceled(cancellationToken);
                return;
            }

            if (args.Error is not null)
            {
                completion.TrySetException(ToUnavailable(args.Error, SpeechRecognitionFailureContext.Recognition));
                return;
            }

            var finalText = args.Result?.Text?.Trim();
            if (!string.IsNullOrWhiteSpace(finalText))
            {
                completion.TrySetResult(finalText);
                return;
            }

            if (!string.IsNullOrWhiteSpace(bestCandidate))
            {
                completion.TrySetResult(bestCandidate);
                return;
            }

            var message = heardAudio || args.BabbleTimeout
                ? "检测到麦克风声音，但未识别出文字；请确认语音语言与 Windows 语音包一致"
                : "没有检测到麦克风声音；请检查默认输入设备和麦克风权限";
            completion.TrySetException(new SpeechRecognitionUnavailableException(message));
        };

        recognizer.SpeechHypothesized += hypothesized;
        recognizer.SpeechRecognitionRejected += rejected;
        recognizer.SpeechRecognized += recognized;
        recognizer.AudioLevelUpdated += audioLevelUpdated;
        recognizer.RecognizeCompleted += completed;
        try
        {
            recognizer.RecognizeAsync(RecognizeMode.Single);
            return await AwaitCompletionWithCancellationAsync(
                completion,
                recognizer.RecognizeAsyncCancel,
                cancellationToken).ConfigureAwait(false);
        }
        finally
        {
            recognizer.SpeechHypothesized -= hypothesized;
            recognizer.SpeechRecognitionRejected -= rejected;
            recognizer.SpeechRecognized -= recognized;
            recognizer.AudioLevelUpdated -= audioLevelUpdated;
            recognizer.RecognizeCompleted -= completed;
        }
    }

    internal static async Task<T> AwaitCompletionWithCancellationAsync<T>(
        TaskCompletionSource<T> completion,
        Action cancelRecognition,
        CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(completion);ArgumentNullException.ThrowIfNull(cancelRecognition);
        using var registration=cancellationToken.Register(() =>
        {
            try{cancelRecognition();}
            catch(Exception)
            {
                // Cancellation must still complete even when the recognizer has
                // already stopped or its audio device disappeared.
            }
            finally{completion.TrySetCanceled(cancellationToken);}
        });
        return await completion.Task.ConfigureAwait(false);
    }

    private static SpeechRecognitionUnavailableException ToUnavailable(
        Exception exception,
        SpeechRecognitionFailureContext context) =>
        exception as SpeechRecognitionUnavailableException ??
        new SpeechRecognitionUnavailableException(
            SpeechRecognitionFailureMapper.FromException(exception, context),
            exception);
}

public static class SpeechRecognizerLanguageSelector
{
    public static CultureInfo? SelectBestCulture(
        string? requestedLanguage,
        IEnumerable<CultureInfo> installedCultures,
        params CultureInfo[] systemCultures)
    {
        ArgumentNullException.ThrowIfNull(installedCultures);
        var installed = installedCultures
            .Where(culture => culture is not null)
            .DistinctBy(culture => culture.Name, StringComparer.OrdinalIgnoreCase)
            .ToArray();
        if (installed.Length == 0)
            return null;

        var normalizedLanguage = requestedLanguage?.Trim();
        var followsSystem = string.IsNullOrWhiteSpace(normalizedLanguage) ||
                            string.Equals(normalizedLanguage, "system", StringComparison.OrdinalIgnoreCase);
        var requested = followsSystem
            ? systemCultures.Where(culture => culture is not null).ToArray()
            : TryCreateCulture(normalizedLanguage!);

        foreach (var culture in requested)
        {
            var exact = installed.FirstOrDefault(candidate =>
                string.Equals(candidate.Name, culture.Name, StringComparison.OrdinalIgnoreCase));
            if (exact is not null)
                return exact;

            var family = installed.FirstOrDefault(candidate => SameLanguageFamily(candidate, culture));
            if (family is not null)
                return family;
        }

        // Never listen with an unrelated language merely because it happens
        // to be the first installed recognizer. SAPI will appear to hear the
        // microphone while producing no useful text (for example, en-US for
        // spoken Chinese), which is worse than an actionable setup error.
        return null;
    }

    private static CultureInfo[] TryCreateCulture(string language)
    {
        try
        {
            return [CultureInfo.GetCultureInfo(language)];
        }
        catch (CultureNotFoundException)
        {
            return [];
        }
    }

    private static bool SameLanguageFamily(CultureInfo candidate, CultureInfo requested)
    {
        if (candidate.IsNeutralCulture || requested.IsNeutralCulture)
            return string.Equals(
                candidate.TwoLetterISOLanguageName,
                requested.TwoLetterISOLanguageName,
                StringComparison.OrdinalIgnoreCase);

        return !string.IsNullOrEmpty(candidate.Parent.Name) &&
               string.Equals(candidate.Parent.Name, requested.Parent.Name, StringComparison.OrdinalIgnoreCase);
    }
}
