"use client";

import { ArrowLeft, ArrowRight, Eye, GraduationCap, Network, RefreshCw, Scale, SquareTerminal } from "lucide-react";
import { useState, type ReactNode } from "react";

import { Button } from "@/components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { useSettled } from "@/lib/settled";
import { useLocalFlag } from "@/lib/use-local-flag";
import { cn } from "@/lib/utils";

const SEEN_KEY = "omniforge.method-guide.v1";

interface Step {
  icon: ReactNode;
  title: string;
  lead: string;
  points: { term: string; text: string }[];
  where: string;
}

const STEPS: Step[] = [
  {
    icon: <Network />,
    title: "Une usine, pas un chat",
    lead: "Vous confiez une demande ; un orchestrateur déterministe la fait traverser une chaîne de production logicielle et distribue le travail à des agents spécialisés.",
    points: [
      { term: "Orchestrateur", text: "une machine à états : ingestion → plan → architecture → TDD → audit → livraison. C'est lui — pas les agents — qui détient les verdicts (tests, lint, sécurité, commits)." },
      { term: "Agents", text: "Product Owner, Architecte, QA, Développeurs, Auditeur sécurité, Relecteur. Chacun a un rôle, des outils et des droits d'écriture limités." },
      { term: "Routage multi-modèles", text: "chaque type de tâche part vers le palier de modèle adapté à son coût et à sa difficulté." },
    ],
    where: "Graphe « Agents » : le nœud entouré d'un halo détient le jeton de parole.",
  },
  {
    icon: <Eye />,
    title: "La boucle Observer → Réfléchir → Agir",
    lead: "Chaque agent travaille par petites itérations vérifiables, jamais en un seul jet.",
    points: [
      { term: "Réfléchir", text: "l'agent raisonne et annonce ce qu'il va faire." },
      { term: "Agir", text: "il appelle un outil : lire ou écrire un fichier, lancer une commande dans la sandbox, exécuter les tests." },
      { term: "Observer", text: "le résultat de l'outil revient et nourrit l'itération suivante, jusqu'à ce que l'agent conclue avec un niveau de confiance." },
    ],
    where: "Volet « Méthode » : l'étape en cours de la boucle s'allume à chaque événement.",
  },
  {
    icon: <RefreshCw />,
    title: "Le cycle TDD est obligatoire",
    lead: "Aucune ligne de code métier n'est écrite avant ses tests, et c'est l'orchestrateur qui exécute les tests dans la sandbox.",
    points: [
      { term: "RED", text: "l'agent QA écrit les tests. Ils doivent échouer, pour la bonne raison. Ils sont ensuite verrouillés." },
      { term: "GREEN", text: "le développeur écrit le code minimal. En cas d'échec, un modèle critique diagnostique la trace et le développeur réessaie : c'est la boucle d'auto-correction." },
      { term: "REFACTOR", text: "lint puis nettoyage, conservé uniquement si les tests restent verts." },
    ],
    where: "Volet « Méthode » : l'anneau TDD, les verdicts de test et le diagnostic d'auto-correction.",
  },
  {
    icon: <Scale />,
    title: "Vous choisissez le niveau de supervision",
    lead: "À chaque point de contrôle, un arbre de décision détermine si les agents continuent seuls ou vous attendent.",
    points: [
      { term: "HITL", text: "pause obligatoire aux étapes clés : plan, architecture, plan de test, merge request." },
      { term: "HOTL", text: "autonomie, avec pause uniquement sur exception : confiance trop basse, règle de sécurité violée, tests toujours rouges après N itérations." },
      { term: "Autonome", text: "aucune attente ; une exception bloquante fait échouer le run plutôt que de passer en force." },
    ],
    where: "Le mode se règle par projet, par étape, et se change en cours de run depuis l'en-tête.",
  },
  {
    icon: <SquareTerminal />,
    title: "Lire le Mission Control",
    lead: "Tout ce qui se passe est un événement horodaté : rien n'est caché, tout est rejouable.",
    points: [
      { term: "Graphes", text: "qui parle (agents) et où en est chaque tâche (dépendances, chemin critique)." },
      { term: "Terminal et workspace", text: "les commandes exécutées dans la sandbox, les fichiers et leurs diffs en direct." },
      { term: "Portail de validation", text: "quand une décision vous revient, une fenêtre présente la proposition ; répondez par écrit ou à la voix." },
    ],
    where: "Onglets Terminal, Workspace, Journal, Plan et Livrable sous le graphe.",
  },
];

/** First-visit walkthrough of the agentic method; re-openable at any time. */
export function MethodGuide({ className }: { className?: string }) {
  const [seen, setSeen] = useLocalFlag(SEEN_KEY, true);
  const [requested, setRequested] = useState(false);
  const [index, setIndex] = useState(0);
  const settled = useSettled();
  // First visit: open by itself, once the page underneath has hydrated.
  const open = requested || (settled && !seen);
  const step = STEPS[index];
  const last = index === STEPS.length - 1;

  const close = () => {
    setSeen(true);
    setRequested(false);
    setIndex(0);
  };

  return (
    <>
      <Button variant="outline" size="sm" className={className} onClick={() => setRequested(true)}>
        <GraduationCap /> Comprendre la méthode
      </Button>
      <Dialog open={open} onOpenChange={(next) => (next ? setRequested(true) : close())}>
        <DialogContent size="lg" aria-describedby="method-guide-lead">
          <DialogHeader>
            <p className="text-[11px] font-semibold tracking-wider text-accent uppercase">
              La méthode agentique · {index + 1} / {STEPS.length}
            </p>
            <DialogTitle className="mt-1 flex items-center gap-2 text-lg [&_svg]:size-5 [&_svg]:text-accent">
              {step.icon}
              {step.title}
            </DialogTitle>
            <DialogDescription id="method-guide-lead">{step.lead}</DialogDescription>
          </DialogHeader>
          <DialogBody className="space-y-3">
            <dl className="space-y-2.5">
              {step.points.map((point) => (
                <div key={point.term} className="rounded-md border border-line bg-raised/50 p-3">
                  <dt className="text-sm font-semibold text-ink">{point.term}</dt>
                  <dd className="mt-0.5 text-sm leading-relaxed text-dim">{point.text}</dd>
                </div>
              ))}
            </dl>
            <p className="rounded-md border border-accent/25 bg-accent/8 px-3 py-2 text-xs text-accent">
              Où le voir : {step.where}
            </p>
          </DialogBody>
          <DialogFooter className="justify-between">
            <div className="flex items-center gap-1.5" aria-hidden>
              {STEPS.map((candidate, dot) => (
                <span
                  key={candidate.title}
                  className={cn("h-1.5 rounded-full transition-all", dot === index ? "w-5 bg-accent" : "w-1.5 bg-line-strong")}
                />
              ))}
            </div>
            <div className="flex items-center gap-2">
              <Button variant="ghost" onClick={close}>
                Passer
              </Button>
              <Button variant="secondary" disabled={index === 0} onClick={() => setIndex((value) => value - 1)}>
                <ArrowLeft /> Précédent
              </Button>
              {last ? (
                <Button variant="primary" onClick={close}>
                  C&apos;est compris
                </Button>
              ) : (
                <Button variant="primary" onClick={() => setIndex((value) => value + 1)}>
                  Suivant <ArrowRight />
                </Button>
              )}
            </div>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
