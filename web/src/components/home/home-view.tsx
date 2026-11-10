"use client";

import { ServerCrash } from "lucide-react";

import { NewRunForm } from "@/components/home/new-run-form";
import { RunsList } from "@/components/home/runs-list";
import { EmptyState, Panel } from "@/components/ui/panel";
import { API_URL } from "@/lib/api";
import { useMarkSettled } from "@/lib/settled";
import { useMeta } from "@/lib/use-meta";

export function HomeView() {
  const { meta, error } = useMeta();
  useMarkSettled();
  return (
    <main className="mx-auto w-full max-w-[1500px] flex-1 p-4">
      <div className="mb-4">
        <h1 className="text-lg font-semibold tracking-tight text-ink">Mission Control</h1>
        <p className="mt-0.5 max-w-3xl text-sm text-dim">
          Confiez une demande à l&apos;usine : elle est classée, planifiée, conçue, implémentée en TDD, auditée puis
          livrée en merge request — sous le niveau de supervision que vous choisissez.
        </p>
      </div>
      <div className="grid items-start gap-4 xl:grid-cols-[minmax(0,1fr)_26rem]">
        {meta ? (
          <NewRunForm meta={meta} />
        ) : (
          <Panel className="min-h-64">
            {error ? (
              <EmptyState icon={<ServerCrash />} title="Backend injoignable">
                L&apos;API OmniForge ne répond pas sur <span className="font-mono text-dim">{API_URL}</span>. Nouvelle
                tentative automatique dans quelques secondes.
              </EmptyState>
            ) : (
              <EmptyState title="Connexion à la factory…" />
            )}
          </Panel>
        )}
        <RunsList simulated={meta?.llm_backend === "simulated"} />
      </div>
    </main>
  );
}
