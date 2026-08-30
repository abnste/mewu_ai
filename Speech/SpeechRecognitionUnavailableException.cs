namespace mewu_ai_Assistant.Speech;

public sealed class SpeechRecognitionUnavailableException(string userMessage,Exception? innerException=null) : Exception(userMessage,innerException);
