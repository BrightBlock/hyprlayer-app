// @vitest-environment jsdom
import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

const s = vi.hoisted(() => ({ deliverReview: vi.fn() }));
vi.mock("@/features/session/sessionStore", () => s);
vi.mock("@/lib/queries", () => ({
  useMarkReviewRoundDelivered: () => ({ mutateAsync: vi.fn(async () => undefined) }),
}));
vi.mock("@/lib/appNav", () => ({ closeReview: vi.fn() }));

import { useReviewDelivery } from "./useReviewDelivery";

describe("useReviewDelivery", () => {
  it("sends a round once when two Resend buttons are pressed", async () => {
    let finish!: (v: "sent") => void;
    s.deliverReview.mockImplementation(() => new Promise((r) => (finish = r)));
    const header = renderHook(() => useReviewDelivery("ws1", null, "Claude"));
    const checklist = renderHook(() => useReviewDelivery("ws1", null, "Claude"));
    const round = { number: 3, commentCount: 2 };

    await act(async () => {
      void header.result.current.deliver(round, "fix these");
    });
    await act(async () => {
      void checklist.result.current.deliver(round, "fix these");
    });
    await act(async () => finish("sent"));

    expect(s.deliverReview).toHaveBeenCalledTimes(1);
  });
});
