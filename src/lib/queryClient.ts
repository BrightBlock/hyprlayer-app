import { MutationCache, QueryCache, QueryClient } from "@tanstack/react-query";

import { log } from "./log";

export const queryClient = new QueryClient({
  // The key and the error only: variables and data carry keys and prompts.
  queryCache: new QueryCache({
    onError: (error, query) => log.warn("query", `${String(query.queryKey[0])} failed`, error),
  }),
  mutationCache: new MutationCache({
    onError: (error, _variables, _context, mutation) =>
      log.warn("mutation", `${String(mutation.options.mutationKey?.[0] ?? "a mutation")} failed`, error),
  }),
  defaultOptions: {
    queries: {
      retry: false,
      refetchOnWindowFocus: true,
      staleTime: 5_000,
    },
  },
});
