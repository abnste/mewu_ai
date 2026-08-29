using System.Windows.Media.Imaging;
using mewu_ai_Assistant.Models;
namespace mewu_ai_Assistant.OCR;
public interface IOcrService { Task<OcrDocument> RecognizeAsync(BitmapSource image,CancellationToken cancellationToken); }
