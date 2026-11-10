# 4. IHM — Mission Control

L'interface n'est pas un chat. Elle répond à trois besoins : **piloter** un run (lancer,
superviser, décider), **comprendre** ce que font les agents pendant qu'ils le font, et
**vérifier** ce qui est produit (terminal, fichiers, diffs, merge request).

Pile : Next.js (App Router, TypeScript), Tailwind CSS, composants de style shadcn/ui bâtis
sur Radix, React Flow (`@xyflow/react`) pour les graphes. Interface en français.

## 4.1 Principe : la même projection que le backend

L'IHM ne maintient pas d'état métier propre. Elle reçoit le journal d'événements du run et
lui applique un réducteur TypeScript qui reproduit celui du backend
(`web/src/lib/reducer.ts` ↔ `backend/src/orchestrator/state.rs`). Ce qu'elle affiche est
donc, par construction, ce que contient la piste d'audit.

```text
                      ┌──────────────── navigateur ────────────────┐
 GET /api/runs/{id}   │                                            │
 ───────────────────▶ │  use-run-stream.ts                         │
 WS  /stream?since=N  │   ├─ rejeu + direct, reconnexion avec le   │
 ◀─────────────────── │   │  dernier numéro de séquence reçu       │
 (repli : /events)    │   └─ événements ──▶ reducer.ts ──▶ état    │
                      │                                    │       │
 POST /gates/…/resolve│   graphe · méthode · terminal · journal ◀──┘
 PUT  /supervision    │   portail de validation · sélecteur        │
 POST /terminal  ◀─── │   (commandes : lib/api.ts)                 │
                      └────────────────────────────────────────────┘
```

Si le WebSocket ne peut pas s'ouvrir, le flux bascule sur une interrogation périodique de
`/events?since=` : mêmes événements, même réducteur.

## 4.2 Vues

### Accueil — lancer et retrouver des runs

```text
┌─ OmniForge AI ───────────────────────────────────────── [ Comprendre la méthode ] ─┐
│                                                                                     │
│  NOUVEAU RUN                                    RUNS                                │
│  ┌───────────────────────────────────────┐     ┌──────────────────────────────────┐ │
│  │ Exemples : [Micro-tâche] [Epic] [PRD] │     │ ● Endpoint JWT    terminé  1/1   │ │
│  │ ┌───────────────────────────────────┐ │     │ ◐ Réservation     en cours 3/7   │ │
│  │ │ Décrivez la demande…              │ │     │ ⏸ Facturation     validation ⚑1  │ │
│  │ └───────────────────────────────────┘ │     │ ✕ Utilitaire      annulé         │ │
│  │ ou déposer un fichier .md .txt .pdf   │     └──────────────────────────────────┘ │
│  │ Niveau : (auto) 1 2 3                 │                                          │
│  │ ┌─ Supervision ─────────────────────┐ │     APERÇU DU PLAN (bouton « Analyser ») │
│  │ │ [HITL] [HOTL] [Autonome]          │ │     niveau détecté · arbre des tâches    │
│  │ │ réglages avancés ▾                │ │     dépendances · risques · sprints      │
│  │ └───────────────────────────────────┘ │                                          │
│  │ [ Analyser ]            [ Lancer ▶ ]  │                                          │
│  └───────────────────────────────────────┘                                          │
└─────────────────────────────────────────────────────────────────────────────────────┘
```

« Analyser » appelle `POST /api/plan` : classification et décomposition sans rien exécuter.

### Mission Control — suivre et superviser un run

```text
┌─ Titre du run ── ● en cours ── ingestion ▸ plan ▸ archi ▸ [IMPLÉMENTATION] ▸ audit ▸ livraison ─┐
│  Supervision : [HITL|HOTL|Auto] ⚙      23 appels · 23 k tokens · 0,08 $      [ Annuler ]       │
├──────────────────────────────────────────────────────────────┬────────────────────────────────┤
│  [ Agents ]  [ Tâches ]                                      │  MÉTHODE                        │
│                                                              │                                 │
│            ┌──────────────┐        ┌──────────┐              │  Observe → Think → Act          │
│            │ Product Owner│        │Architecte│              │   (●)──▶( )──▶( )   « le dev    │
│            └──────┬───────┘        └────┬─────┘              │    lit les tests en échec »     │
│   ┌────────┐      │   ┌──────────────┐  │    ┌───────────┐   │                                 │
│   │Humain ⚑│──────┼──▶│ ORCHESTRATEUR│◀─┼───▶│  Sandbox  │   │  Cycle TDD      tentative 2/4   │
│   └────────┘      │   └──────┬───────┘  │    └───────────┘   │   RED ─▶ [GREEN] ─▶ REFACTOR    │
│            ┌──────┴───────┐  │   ┌──────┴─────┐              │   diagnostic : « exp jamais     │
│            │ ◉ Dev backend│◀═╧══▶│     QA     │              │   comparé à l'heure courante »  │
│            │ GREEN · T1   │      └────────────┘              │                                 │
│            └──────────────┘   Auditeur    Relecteur          │  Arbre de supervision           │
│   ◉ = détient le jeton de parole (arêtes animées)            │   HOTL ─ exception ? ─ non ─▶ ✓ │
│                                                              │                                 │
│                                                              │  Patterns · Routage des modèles │
├──────────────────────────────────────────────────────────────┴────────────────────────────────┤
│  [ Terminal ]  [ Workspace ]  [ Journal ]  [ Plan ]  [ Livrable ]                              │
│  orchestrateur $ pytest -q --tb=short                                               exit 1    │
│  5 passed, 1 failed                                                                            │
│  vous $ ▌                                                                                      │
└────────────────────────────────────────────────────────────────────────────────────────────────┘
```

| Zone | Contenu | Événements exploités |
|---|---|---|
| En-tête | Statut, étape du pipeline, bascule de supervision à chaud, coût et tokens | `run.status_changed`, `run.phase_changed`, `llm.call`, `supervision.changed` |
| Graphe **Agents** | Orchestrateur au centre, les sept rôles, le superviseur humain, la sandbox. L'agent actif est mis en évidence, avec sa phase, sa tâche et sa dernière pensée ; le nœud humain pulse quand un portail attend | `agent.turn_started`, `agent.turn_ended`, `agent.thought`, `gate.requested` |
| Graphe **Tâches** | Feuilles du plan disposées par vagues, arêtes de dépendance, couleur selon l'état (rouge, vert, refactor, terminé, échec), chemin critique, sprint et risque | `plan.created`, `task.status_changed` |
| Volet **Méthode** | Ce qui se passe, expliqué pendant que cela se passe : voir §4.3 | `agent.*`, `action`, `observation`, `tdd.*`, `gate.evaluated`, `llm.call` |
| **Terminal** | Commandes de l'orchestrateur et des agents avec leur sortie et leur code de retour ; ligne de saisie pour exécuter une commande dans la sandbox tant que le run est actif | `terminal.output`, `action`, `observation` |
| **Workspace** | Arborescence, contenu des fichiers, diff par fichier en direct et diff cumulé | `workspace.file_changed`, `/workspace/*` |
| **Journal** | Chronologie filtrable par agent, type et tâche | tous |
| **Plan** | Arbre HTN, sprints, matrice des risques | `plan.created` |
| **Livrable** | Merge request, architecture, findings de sécurité, violations de politique | `artifact.created`, `security.findings`, `policy.violation` |

### Portail de validation

S'ouvre de lui-même dès qu'un portail attend une décision, et se rouvre depuis un bandeau.

```text
┌─ Validation requise ─ Plan de test — Connexion avec jeton JWT ─────────────────────┐
│ Pourquoi : HITL — validation humaine obligatoire à cette étape clé                 │
│ Confiance 88 %  ▓▓▓▓▓▓▓▓▓░  seuil 70 %                                              │
│ ┌─ Proposition ──────────────────────────────────────────────────────────────────┐ │
│ │ tests/test_jwt_auth.py                                                         │ │
│ │   def test_expired_token_is_rejected(): …                                      │ │
│ │ Exécution RED : 0 réussi, 1 erreur — échec attendu ✓                           │ │
│ └────────────────────────────────────────────────────────────────────────────────┘ │
│ Commentaire  [ écrit ou dicté…                                              ] 🎤   │
│ [ Approuver ]  [ Demander des modifications ]  [ Rejeter et arrêter le run ]       │
└────────────────────────────────────────────────────────────────────────────────────┘
```

Le contenu dépend de la nature de la proposition : arbre du plan, document d'architecture,
fichiers de test et exécution RED, trace de la divergence TDD et dernier diagnostic,
findings de sécurité, merge request avec son diff, dépassement de budget. Les libellés des
boutons viennent du backend : « approuver » ne signifie pas la même chose devant un plan
(« continuer ») que devant une divergence (« accorder des itérations supplémentaires »).

Le commentaire peut être dicté : reconnaissance vocale du navigateur (Web Speech API, en
français). Sur un navigateur qui ne la propose pas, le bouton l'indique et la saisie
clavier reste disponible.

### Sélecteur de supervision

Le même composant sert au lancement d'un run et à la modification en cours de run.

```text
┌─ Supervision ────────────────────────────────────────────────────────────────┐
│ ┌──────────────────┐ ┌──────────────────┐ ┌──────────────────┐               │
│ │ HITL             │ │ HOTL          ●  │ │ Autonome         │               │
│ │ pause à chaque   │ │ pause seulement  │ │ aucune pause ;   │               │
│ │ étape clé        │ │ sur exception    │ │ échec si blocage │               │
│ └──────────────────┘ └──────────────────┘ └──────────────────┘               │
│ Par étape        hérité   HITL   HOTL   Autonome                             │
│   plan             ●       ○      ○       ○                                  │
│   architecture     ●       ○      ○       ○                                  │
│   merge            ○       ●      ○       ○                                  │
│ Seuil de confiance  ───────●──── 70 %      Itérations TDD max  [ 4 ]         │
│ Budget (USD)        [      ]               Sévérité bloquante  [ high ▾ ]    │
└──────────────────────────────────────────────────────────────────────────────┘
```

Il produit exactement l'objet `SupervisionPolicy` attendu par l'API.

## 4.3 Le volet pédagogique

Il rend la méthode lisible en s'appuyant sur les événements réels du run, pas sur une
animation décorative :

| Bloc | Ce qu'il montre | Source |
|---|---|---|
| Boucle Observe → Think → Act | L'étape en cours de l'agent actif et une phrase en clair décrivant son action | Dernier `agent.thought`, `action` ou `observation` |
| Cycle TDD | La phase (RED, GREEN, REFACTOR), le compteur de tentatives, le dernier verdict et le diagnostic d'auto-correction | `tdd.phase_changed`, `tdd.test_result`, `tdd.reflection` |
| Arbre de supervision | Le chemin emprunté lors du dernier point de contrôle : mode, étape clé ou non, exceptions, verdict, motif | `gate.evaluated` |
| Patterns d'orchestration | Celui qui est à l'œuvre : chaînage, routage, parallélisation, orchestrateur-exécutants, évaluateur-optimiseur | Étape du pipeline et agents actifs |
| Routage des modèles | La table nature → palier → modèles, et la ligne du dernier appel avec le motif (escalade, rétrogradation, repli) | `/api/meta`, `llm.call` |

Un parcours guidé en quelques étapes s'affiche à la première visite et se rouvre par le
bouton « Comprendre la méthode ».

## 4.4 Structure du code

```text
web/src/
  app/
    page.tsx                      accueil
    runs/[id]/page.tsx            Mission Control
  components/
    home/                         formulaire de lancement, liste des runs
    mission/
      mission-control.tsx         disposition générale de la vue
      run-header.tsx              statut, pipeline, supervision à chaud, métriques
      agent-graph.tsx             graphe multi-agents (React Flow)
      task-dag.tsx                graphe de dépendances des tâches (React Flow)
      method-panel.tsx            volet pédagogique
      terminal.tsx                terminal de la sandbox
      workspace.tsx               explorateur de fichiers et diffs
      journal.tsx                 chronologie des événements
      deliverable.tsx             merge request, architecture, findings
      gate-portal.tsx             portail de validation
    supervision-selector.tsx      sélecteur HITL / HOTL / Autonome
    method-guide.tsx              parcours guidé
    diff-view.tsx                 rendu d'un diff unifié
    plan-view.tsx                 arbre HTN, sprints, risques
    ui/                           primitives (bouton, dialogue, onglets…)
  lib/
    types.ts                      types des événements, de l'état, du plan, de la politique
    reducer.ts                    projection événements → état
    use-run-stream.ts             WebSocket, reconnexion, repli en interrogation
    use-speech-input.ts           dictée vocale
    api.ts                        client REST typé
```

## 4.5 Configuration

| Variable | Défaut | Rôle |
|---|---|---|
| `NEXT_PUBLIC_API_URL` | `http://localhost:8080` | Adresse de l'API vue par le navigateur |
| `NEXT_PUBLIC_API_KEY` | — | Clé envoyée à l'API si `OMNIFORGE_API_KEY` est définie |

Ces variables sont intégrées au bundle servi au navigateur : la clé d'API y est donc
lisible par quiconque accède à l'IHM. Elle protège une instance contre un accès direct à
l'API, pas contre les utilisateurs de l'interface. Une authentification par utilisateur
relève de la passerelle d'entreprise placée devant le service.

L'origine de l'IHM doit figurer dans `OMNIFORGE_CORS_ORIGINS` côté API, faute de quoi les
écritures et le flux temps réel sont refusés.
