# Local OCR lifetime

Windows uses WinRT OCR, Linux uses the installed local recognizer, and macOS uses Apple Vision. Recognition receives an owned composed image, not permission to recapture the desktop or read arbitrary files. The host retains the original scene, background, plugin revision and cancellation checks before publishing a result.

On macOS, Vision runs in a short-lived helper of the installed Mewu executable. This keeps a failed process-local Vision/Core ML model session from contaminating later captures in the long-lived application. No extra executable or runtime is installed. A strict anonymous-pipe protocol carries bounded RGBA pixels and a validated OCR document with exactly the source dimensions; no image files, credentials, URLs or host handles enter the protocol.

There are at most two direct OCR helpers per application process. Cancellation or the bounded deadline terminates the owned process group. The existing child-process supervisor retains the capacity permit and real IO handles until exit is confirmed; a timeout does not free a still-running worker slot. Failure is reported instead of returning fabricated or partial text. The process lifetime is separate from copying: successful native clipboard publication is still required before quick capture closes.
