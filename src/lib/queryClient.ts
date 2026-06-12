import { QueryClient } from "@tanstack/react-query";

export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      // Tauri IPC is local: retries mask real errors and refetch is cheap.
      retry: false,
      refetchOnWindowFocus: true,
      staleTime: 5_000,
    },
  },
});
