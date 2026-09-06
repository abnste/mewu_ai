// SPDX-FileCopyrightText: 2026 Abner Stephen and contributors
// SPDX-License-Identifier: MPL-2.0
namespace mewu_ai_Assistant.Speech;

public sealed class SpeechRecognitionUnavailableException(string userMessage,Exception? innerException=null) : Exception(userMessage,innerException);
