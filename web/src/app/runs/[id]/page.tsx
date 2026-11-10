import { Suspense } from "react";

import { MissionRoute } from "@/components/mission/mission-route";

export default function RunPage() {
  return (
    <Suspense fallback={<p className="p-6 text-sm text-faint">Chargement du run…</p>}>
      <MissionRoute />
    </Suspense>
  );
}
