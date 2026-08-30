using System.Globalization;
using System.Speech.Recognition;

namespace mewu_ai_Assistant.Speech;

public sealed class WindowsSpeechToTextService : ISpeechToTextService
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
        EventHandler<RecognizeCompletedEventArgs>? completed = null;
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

            if (args.InitialSilenceTimeout || args.BabbleTimeout || args.Result is null ||
                string.IsNullOrWhiteSpace(args.Result.Text))
            {
                completion.TrySetException(new SpeechRecognitionUnavailableException("没有听到语音，请重试"));
                return;
            }

            completion.TrySetResult(args.Result.Text.Trim());
        };

        recognizer.RecognizeCompleted += completed;
        try
        {
            recognizer.RecognizeAsync(RecognizeMode.Single);
            using var registration = cancellationToken.Register(
                () =>
                {
                    try
                    {
                        recognizer.RecognizeAsyncCancel();
                    }
                    catch (Exception)
                    {
                        // The single recognition may already have completed.
                        completion.TrySetCanceled(cancellationToken);
                    }
                });
            return await completion.Task.ConfigureAwait(false);
        }
        finally
        {
            recognizer.RecognizeCompleted -= completed;
        }
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

        return followsSystem ? installed[0] : null;
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
