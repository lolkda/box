import { createContext, useCallback, useContext, useMemo, useState, type ReactNode } from "react";
import * as ToastPrimitive from "@radix-ui/react-toast";
import { CheckCircle2, Info, X, XCircle } from "lucide-react";
import { cn } from "../../lib/utils";

type Tone = "success" | "error" | "info";

interface ToastOptions {
  title: string;
  description?: string;
  tone?: Tone;
}

interface ToastItem extends ToastOptions {
  id: number;
}

const ToastContext = createContext<{ toast: (options: ToastOptions) => void } | null>(null);

let toastId = 0;

const toneStyles: Record<Tone, string> = {
  success: "border-success/25 bg-success-soft text-success",
  error: "border-danger/25 bg-danger-soft text-danger",
  info: "border-info/25 bg-info-soft text-info",
};

const toneIcons: Record<Tone, ReactNode> = {
  success: <CheckCircle2 size={18} />,
  error: <XCircle size={18} />,
  info: <Info size={18} />,
};

export function ToastProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<ToastItem[]>([]);

  const remove = useCallback((id: number) => {
    setItems((current) => current.filter((item) => item.id !== id));
  }, []);

  const toast = useCallback(
    (options: ToastOptions) => {
      const id = ++toastId;
      setItems((current) => [...current, { id, tone: "info", ...options }]);
      window.setTimeout(() => remove(id), 5000);
    },
    [remove],
  );

  const value = useMemo(() => ({ toast }), [toast]);

  return (
    <ToastContext.Provider value={value}>
      <ToastPrimitive.Provider swipeDirection="right">
        {children}
        {items.map((item) => (
          <ToastPrimitive.Root
            key={item.id}
            open
            onOpenChange={(open) => {
              if (!open) remove(item.id);
            }}
            duration={5000}
            className={cn(
              "flex w-[min(22rem,calc(100vw-2rem))] items-start gap-3 rounded-xl border p-3.5 shadow-xl",
              "data-[state=open]:animate-box-slide",
              toneStyles[item.tone || "info"],
            )}
          >
            <span className="mt-0.5 shrink-0">{toneIcons[item.tone || "info"]}</span>
            <div className="min-w-0 flex-1">
              <ToastPrimitive.Title className="text-sm font-semibold">{item.title}</ToastPrimitive.Title>
              {item.description ? (
                <ToastPrimitive.Description className="mt-0.5 text-xs leading-5 opacity-80">
                  {item.description}
                </ToastPrimitive.Description>
              ) : null}
            </div>
            <ToastPrimitive.Close className="shrink-0 rounded-md p-0.5 opacity-60 hover:opacity-100" aria-label="关闭通知">
              <X size={15} />
            </ToastPrimitive.Close>
          </ToastPrimitive.Root>
        ))}
        <ToastPrimitive.Viewport className="fixed bottom-[calc(5.5rem+var(--safe-bottom))] right-3 z-[80] flex max-w-[calc(100vw-1.5rem)] flex-col gap-2 outline-none" />
      </ToastPrimitive.Provider>
    </ToastContext.Provider>
  );
}

export function useToast() {
  const context = useContext(ToastContext);
  if (!context) throw new Error("useToast must be used within ToastProvider");
  return context.toast;
}
