# Linux 本机文字识别

Linux 适配器通过已安装的 Tesseract 4.1 或 5 C API 识别宿主提供的选区图像，不运行命令行程序，不下载模型，也不采集屏幕。独立的系统组件与语言包不随 Mewu 发布。

目前支持 Debian/Ubuntu 风格的固定库目录：x86_64 的 `/usr/lib/x86_64-linux-gnu`，aarch64 的 `/usr/lib/aarch64-linux-gnu`。优先选择 `libtesseract.so.5`；只有该路径确实缺失时才选择 `libtesseract.so.4`，并核对实际 `TessVersion` 分别为 5.x 或 4.1.x。已有但不安全、不可读、链接损坏或版本不匹配的优先库会直接失败，不通过回退绕过检查。

5 的固定模型目录为 `/usr/share/tesseract-ocr/5/tessdata`，4.1 为 `/usr/share/tesseract-ocr/4.00/tessdata`；各自也接受固定的 `/usr/share/tessdata`。英语模型必须存在；简体、繁体模型只有实际安装时才加入识别语言。已有但不安全的模型目录或英语文件不会被静默跳过。未安装中文模型不能据此声明中文识别通过。其他发行版路径不自动搜索，缺失组件时给出明确错误。Ubuntu 22.04 使用系统的 `libtesseract4` 与 `tesseract-ocr-eng`，保留原有构建系统与 GLIBC 基线。

库、模型及路径父目录必须由当前命名空间中可验证的 root 所有，普通文件和目录不能被 group/other 写入。符号链接的原始路径和解析路径均检查；模型目标限制在固定模型目录，单文件最多 64 MiB。库使用保留的文件描述符加载，模型文件描述符与 dev/ino/size/mtime 在初始化后再核对。此约束防止普通用户替换资源，不承诺抵御宿主 root 对已信任系统组件的修改。

用户命名空间显示的 UID 65534 不等于已验证的宿主 root。普通宿主适配器不会放宽这一要求；以后若 Code worker 使用不同命名空间，需要独立的、不可由调用者伪造的外层资源证明。命令沙箱探测不代替桌面宿主测试。

图像遵守共享的 3200 万源像素与 400 万识别像素限制，透明部分合成白色，居中等比缩放，然后将 BGRA 转为 Tesseract 的 RGB。词边界映射回原选区，并由共享文档校验器核查行数、词数、文本长度与坐标。

整个调用使用同一个 15 秒预算，最多两个阻塞 worker。取消回调和原生 monitor 使用剩余预算；不足一毫秒直接超时，不能给原生接口传入表示无限等待的零。初始化本身不可强制取消；超时或取消后仍保留 native API、monitor、回调上下文、像素及宿主租约，直到同步调用真正返回。取消和超时不证明 worker 已完成。

原生验收需要显式安装系统组件、英语模型及本地字体，然后在真实 Linux 上运行：

```sh
cargo test -p mewu-desktop --bin mewu-desktop --locked real_linux_tesseract_recognizes_known_english_and_blank -- --ignored --nocapture --test-threads=1
```

该测试用现有绘图器生成已知英文，检查真实文本和词边界，以及空白图像结果。它不读取桌面，不证明中文识别或完整 GUI 功能。普通源码测试覆盖提前取消、原始截止时间和有界 UTF-8 读取。Windows 编译或 Linux 交叉类型检查不能代替这一原生验收。

接口和所有权依据：[Tesseract 5.5.1 C API](https://github.com/tesseract-ocr/tesseract/blob/5.5.1/include/tesseract/capi.h)、[对应实现](https://github.com/tesseract-ocr/tesseract/blob/5.5.1/src/api/capi.cpp)、[显式模型路径优先级](https://github.com/tesseract-ocr/tesseract/blob/5.5.1/src/ccutil/ccutil.cpp)，以及 [4.1.1 C API](https://github.com/tesseract-ocr/tesseract/blob/4.1.1/src/api/capi.h)、[对应实现](https://github.com/tesseract-ocr/tesseract/blob/4.1.1/src/api/capi.cpp) 与 [4.1.1 显式模型路径优先级](https://github.com/tesseract-ocr/tesseract/blob/4.1.1/src/ccutil/mainblk.cpp)。仅使用两者签名一致的函数和不透明句柄；不依赖内部结构布局。实现为 Mewu 的 MPL-2.0 代码，没有复制第三方库源码或模型。
