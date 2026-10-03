import { useState } from "react";

import { closeReview } from "@/lib/appNav";
import type { Session } from "@/lib/bindings";
import { plural } from "@/lib/format";
import { useMarkReviewRoundDelivered } from "@/lib/queries";
import { useToast } from "@/lib/toast";
import { deliverReview } from "@/features/session/sessionStore";

export type DeliverableRound = { number: number; commentCount: number };

// Rounds waiting behind a running turn; Resend must not queue a second copy.
const queuedRounds = new Set<string>();
// Rounds whose text reached the harness but were never marked; Resend only re-marks them.
const sentRounds = new Set<string>();

// A round is delivered only once the harness has the text; a queued send waits for its flush.
export function useReviewDelivery(workstreamId: string, implement: Session | null, agentLabel: string) {
  const markDelivered = useMarkReviewRoundDelivered();
  const [delivering, setDelivering] = useState(false);

  const failed = (retry: () => Promise<void>) =>
    useToast.getState().show("Not delivered", `${agentLabel} didn't receive the review.`, {
      tone: "bad",
      action: { label: "Resend", onClick: retry },
    });

  const delivered = async (round: DeliverableRound) => {
    await markDelivered.mutateAsync({ workstreamId, round: round.number });
    useToast.getState().show(`${plural(round.commentCount, "comment")} sent to ${agentLabel}`, "", {
      action: { label: "Back to chat", onClick: closeReview },
      timed: true,
    });
  };

  // Once the text went out, Resend only re-marks the round, so it is never sent twice.
  const remark = async (round: DeliverableRound): Promise<void> => {
    const key = `${workstreamId}:${round.number}`;
    sentRounds.add(key);
    try {
      await delivered(round);
      sentRounds.delete(key);
    } catch {
      failed(() => remark(round));
    }
  };

  const deliver = async (round: DeliverableRound, message: string): Promise<void> => {
    const key = `${workstreamId}:${round.number}`;
    if (queuedRounds.has(key)) return;
    if (sentRounds.has(key)) return remark(round);
    setDelivering(true);
    try {
      const outcome = await deliverReview(workstreamId, implement, message, () => {
        queuedRounds.delete(key);
        void remark(round);
      });
      if (outcome === "queued") queuedRounds.add(key);
      if (outcome === "sent") await remark(round);
    } catch {
      failed(() => deliver(round, message));
    } finally {
      setDelivering(false);
    }
  };

  return { deliver, delivering };
}
