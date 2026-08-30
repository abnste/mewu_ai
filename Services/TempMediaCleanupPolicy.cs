namespace mewu_ai_Assistant.Services;

internal static class TempMediaCleanupPolicy
{
    internal static string? GetBlockReason(bool isCaptureActive,int activeLeaseCount=0)
    {
        if(isCaptureActive)return "屏幕助手仍在使用截图或录屏文件，请先关闭覆盖层后再清理临时媒体";
        return activeLeaseCount>0
            ?$"仍有 {activeLeaseCount} 个临时媒体引用正在录制、预览、保存或贴视频，请关闭相关窗口或等待操作完成后再清理"
            :null;
    }
}
