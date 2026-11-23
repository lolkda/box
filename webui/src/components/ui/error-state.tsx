import { AlertTriangle, RefreshCw } from "lucide-react";
import { Button } from "./button";
import { getErrorDetail, getErrorMessage } from "../../lib/utils";

export function ErrorState({
  error,
  onRetry,
  title = "加载失败",
}: {
  error: unknown;
  onRetry?: () => void;
  title?: string;
}) {
  const detail = getErrorDetail(error);
  return (
    <div className="rounded-2xl border border-danger/20 bg-danger-soft/60 p-4" role="alert">
      <div className="flex items-start gap-3">
        <AlertTriangle size={18} className="mt-0.5 shrink-0 text-danger" />
        <div className="min-w-0 flex-1">
          <p className="text-sm font-semibold text-danger">{title}</p>
          <p className="mt-1 text-xs leading-5 text-fg">{getErrorMessage(error)}</p>
          {detail ? <pre className="mt-2 max-h-32 overflow-auto whitespace-pre-wrap break-all text-[11px] text-fg-muted">{detail}</pre> : null}
        </div>
      </div>
      {onRetry ? (
        <Button size="sm" variant="secondary" className="mt-3" onClick={onRetry} icon={<RefreshCw size={14} />}>
          重试
        </Button>
      ) : null}
    </div>
  );
}
