import { useEffect, useRef, useState } from "react";

export type AsyncState<T> =
  | { status: "loading" }
  | { status: "error"; message: string }
  | { status: "ready"; data: T };

/**
 * Minimal fetch-state hook: every remote read in the app renders one of
 * loading / error / ready — a failed query must surface as an error, never
 * as an empty-but-confident grid (spec AC11).
 */
export function useAsync<T>(
  fn: () => Promise<T>,
  deps: readonly unknown[],
): AsyncState<T> {
  const [state, setState] = useState<AsyncState<T>>({ status: "loading" });
  const runRef = useRef(0);
  useEffect(() => {
    const run = ++runRef.current;
    setState({ status: "loading" });
    fn().then(
      (data) => {
        if (runRef.current === run) setState({ status: "ready", data });
      },
      (err: unknown) => {
        if (runRef.current === run)
          setState({
            status: "error",
            message: err instanceof Error ? err.message : String(err),
          });
      },
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);
  return state;
}
