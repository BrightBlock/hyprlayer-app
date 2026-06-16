import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";

import { attachEventListeners } from "./lib/events";
import { queryClient } from "./lib/queryClient";
import { router } from "./router";
import { initTheme } from "./lib/theme";
import "./index.css";

// Apply the persisted color theme before the first paint to avoid a flash of
// the default palette.
initTheme();
attachEventListeners();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
    </QueryClientProvider>
  </React.StrictMode>,
);
