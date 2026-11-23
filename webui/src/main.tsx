import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { RouterProvider } from "react-router-dom";
import { AppProviders } from "./app/providers";
import { router } from "./app/router";
import "./styles/index.css";

async function bootstrap() {
  if (import.meta.env.DEV && new URLSearchParams(window.location.search).has("mock")) {
    const { installMockKsu } = await import("./dev/mock-ksu");
    installMockKsu();
  }

  const container = document.getElementById("app");
  if (!container) throw new Error("缺少 #app 容器");

  createRoot(container).render(
    <StrictMode>
      <AppProviders>
        <RouterProvider router={router} />
      </AppProviders>
    </StrictMode>,
  );
}

void bootstrap();
