import { useLayoutEffect, useState } from "react";

import { consumeTabTitleAnimation } from "./tabTitles";

// A freshly generated title fades in. Fading is composited; typing it out repainted the tab per letter.
export function TabTitle({
  workstreamId,
  shellId,
  title,
}: {
  workstreamId: string;
  shellId: number;
  title: string;
}) {
  const [fresh, setFresh] = useState(false);

  useLayoutEffect(() => {
    setFresh(consumeTabTitleAnimation(workstreamId, shellId));
  }, [title, workstreamId, shellId]);

  return fresh ? (
    <span key={title} className="animate-fade-in">
      {title}
    </span>
  ) : (
    <>{title}</>
  );
}
