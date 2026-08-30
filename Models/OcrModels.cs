namespace mewu_ai_Assistant.Models;
public sealed record OcrWord(string Text,double X,double Y,double Width,double Height,double Confidence=1);
public sealed record OcrLine(string Text,double X,double Y,double Width,double Height,IReadOnlyList<OcrWord> Words);
public sealed record OcrDocument(string Text,IReadOnlyList<OcrLine> Lines,string Engine="Windows OCR");
