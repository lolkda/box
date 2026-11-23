import type { ReactNode } from "react";
import * as AccordionPrimitive from "@radix-ui/react-accordion";
import { ChevronDown } from "lucide-react";
import { cn } from "../../lib/utils";

export function Accordion({
  defaultValue,
  children,
  className,
}: {
  defaultValue?: string[];
  children: ReactNode;
  className?: string;
}) {
  return (
    <AccordionPrimitive.Root type="multiple" defaultValue={defaultValue} className={cn("space-y-3", className)}>
      {children}
    </AccordionPrimitive.Root>
  );
}

export function AccordionItem({
  value,
  title,
  description,
  children,
  className,
}: {
  value: string;
  title: ReactNode;
  description?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <AccordionPrimitive.Item
      value={value}
      className={cn("overflow-hidden rounded-2xl border border-border bg-surface shadow-sm", className)}
    >
      <AccordionPrimitive.Header className="flex">
        <AccordionPrimitive.Trigger className="group flex w-full items-center justify-between gap-3 px-4 py-3.5 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/30">
          <span className="min-w-0">
            <span className="block text-[15px] font-semibold text-fg">{title}</span>
            {description ? <span className="mt-0.5 block text-xs text-fg-muted">{description}</span> : null}
          </span>
          <ChevronDown size={18} className="shrink-0 text-fg-muted transition-transform group-data-[state=open]:rotate-180" />
        </AccordionPrimitive.Trigger>
      </AccordionPrimitive.Header>
      <AccordionPrimitive.Content className="overflow-hidden data-[state=closed]:animate-none">
        <div className="border-t border-border px-4 py-4">{children}</div>
      </AccordionPrimitive.Content>
    </AccordionPrimitive.Item>
  );
}
