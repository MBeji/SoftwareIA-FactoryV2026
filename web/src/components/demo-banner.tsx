"use client";

import { Clapperboard } from "lucide-react";

import { Button } from "@/components/ui/button";
import { demoIsOptional, leaveDemo, useDemoMode } from "@/lib/demo";

/** Says plainly that nothing shown here is running right now. */
export function DemoBanner() {
  const demo = useDemoMode();
  if (!demo) return null;
  return (
    <div
      role="note"
      className="mb-4 flex flex-wrap items-start gap-x-3 gap-y-2 rounded-lg border border-violet/30 bg-violet/8 px-3 py-2.5"
    >
      <Clapperboard className="mt-0.5 size-4 shrink-0 text-violet" />
      <div className="min-w-0 flex-1 basis-80 text-sm">
        <p className="font-medium text-ink">Mode démonstration — aucun backend connecté</p>
        <p className="mt-0.5 text-dim">
          Les runs ci-dessous sont des <strong className="font-medium text-ink">enregistrements de vrais runs</strong>,
          rejoués événement par événement. Les portails de validation attendent votre clic, puis la décision
          enregistrée se déroule ; lancer un run, exécuter une commande ou changer la supervision demande un
          backend OmniForge.
        </p>
      </div>
      {demoIsOptional() ? (
        <Button size="sm" onClick={leaveDemo}>
          Revenir au backend
        </Button>
      ) : null}
    </div>
  );
}
