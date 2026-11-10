"use client";

import { Box, Cpu, Factory } from "lucide-react";
import Link from "next/link";

import { MethodGuide } from "@/components/method-guide";
import { Badge } from "@/components/ui/badge";
import { Tooltip } from "@/components/ui/tooltip";
import { useMeta } from "@/lib/use-meta";

export function AppBar() {
  const { meta } = useMeta();
  return (
    <header className="sticky top-0 z-40 flex h-12 shrink-0 items-center justify-between gap-3 border-b border-line bg-panel/95 px-4 backdrop-blur">
      <Link href="/" className="flex items-center gap-2.5 rounded-md pr-2">
        <span className="flex size-7 items-center justify-center rounded-md bg-accent text-accent-ink">
          <Factory className="size-4" />
        </span>
        <span className="text-sm font-semibold tracking-tight text-ink">OmniForge AI</span>
        <span className="hidden text-xs text-faint md:inline">Software factory agentique</span>
      </Link>
      <div className="flex items-center gap-2">
        {meta ? (
          <>
            <Tooltip
              label={
                meta.llm_backend === "simulated"
                  ? "Backend LLM simulé : agents scriptés et déterministes, sandbox et tests bien réels. Aucun appel à un fournisseur."
                  : "Backend LLM : passerelle compatible OpenAI (proxy LiteLLM)."
              }
            >
              <Badge tone={meta.llm_backend === "simulated" ? "warn" : "ok"} tabIndex={0}>
                <Cpu /> LLM : {meta.llm_backend === "simulated" ? "simulé" : "passerelle"}
              </Badge>
            </Tooltip>
            <Tooltip
              label={
                meta.sandbox === "local"
                  ? "Sandbox locale : les commandes des agents s'exécutent sur l'hôte, sans isolation. Démo et tests uniquement."
                  : "Sandbox Docker : un conteneur durci et sans réseau par run."
              }
            >
              <Badge tone={meta.sandbox === "local" ? "warn" : "ok"} tabIndex={0} className="hidden sm:inline-flex">
                <Box /> Sandbox : {meta.sandbox}
              </Badge>
            </Tooltip>
          </>
        ) : null}
        <MethodGuide />
      </div>
    </header>
  );
}
