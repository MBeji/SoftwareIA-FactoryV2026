# 1. Architecture globale

OmniForge AI est une *software factory* agentique : elle reçoit une demande (user story,
epic ou cahier des charges), la planifie, puis la fait réaliser par des agents spécialisés
sous un cycle TDD obligatoire, dans une sandbox, avec un niveau de supervision humaine
réglable. Ce document décrit les composants, leurs frontières et les choix structurants.

> **Lecture rapide.** Le workflow est une machine à états déterministe, pas un agent. Les
> agents sont des exécutants auxquels l'orchestrateur confie des missions bornées. Tout ce
> qui se passe est un événement dans un journal immuable ; l'état affiché en est une
> projection. Les verdicts qui comptent (tests, lint, sécurité, portails) appartiennent à
> l'orchestrateur, jamais aux agents.

## 1.1 Vue d'ensemble

```text
┌────────────────────────────────────────────────────────────────────────────────────────┐
│  IHM « Mission Control »  —  Next.js · Tailwind · shadcn/ui · React Flow        (web/) │
│  graphe multi-agents en direct · volet Méthode · terminal & diffs · portail de         │
│  validation · sélecteur de supervision                                                 │
└───────────────▲───────────────────────────────────────────────┬────────────────────────┘
                │ WebSocket : flux d'événements (replay + direct)│ REST : commandes
┌───────────────┴───────────────────────────────────────────────▼────────────────────────┐
│  API GATEWAY  (axum)                                             backend/src/api.rs     │
│  clé d'API · CORS · upload de documents · /api/runs · /stream · /gates · /terminal      │
├────────────────────────────────────────────────────────────────────────────────────────┤
│  ORCHESTRATION ENGINE                                     backend/src/orchestrator/     │
│                                                                                         │
│   ┌───────────┐  ┌──────────┐  ┌────────────┐  ┌─────────────────┐  ┌───────┐  ┌──────┐ │
│   │ INGESTION │─▶│ PLANNING │─▶│ARCHITECTURE│─▶│ IMPLEMENTATION  │─▶│ AUDIT │─▶│DELIV.│ │
│   │ niveau 1-3│  │ HTN, DAG │  │  ADR, plan │  │ TDD par tâche   │  │ ∥ sec │  │ MR   │ │
│   └───────────┘  └────┬─────┘  └─────┬──────┘  │ RED→GREEN→REFAC.│  │ ∥ rev.│  └──┬───┘ │
│                       ▼              ▼         └───────┬─────────┘  └───┬───┘     ▼     │
│                  ┌────────────────────────────────────────────────────────────────────┐ │
│                  │ GATEKEEPER — matrice de supervision HITL / HOTL / Autonome         │ │
│                  └────────────────────────────────────────────────────────────────────┘ │
│   ┌───────────────────────────────────────────────────────────────────────────────────┐ │
│   │ AGENT RUNTIME — boucle Observe → Think → Act, 7 rôles, moindre privilège          │ │
│   └───────────────────────────────────────────────────────────────────────────────────┘ │
├──────────────────┬──────────────────┬──────────────────┬───────────────┬───────────────┤
│  LLM ROUTER      │  SANDBOX MANAGER │  EVENT STORE     │ MEMORY STORE  │ SÉCURITÉ      │
│  nature → palier │  1 conteneur/run │  journal immuable│ leçons,       │ scanner SAST  │
│  escalade, budget│  sans réseau,    │  SQLite + diffus.│ décisions     │ garde-fou de  │
│  repli           │  capacités nulles│  par run         │ (vecteurs)    │ commandes     │
│  llm/router.rs   │  sandbox/        │  events.rs       │ memory.rs     │ governance/   │
└────────┬─────────┴────────┬─────────┴──────────────────┴───────────────┴───────────────┘
         │ HTTP compatible OpenAI    │ docker run / docker exec
┌────────▼─────────┐        ┌────────▼──────────────────────────────────────────────────┐
│  PROXY LiteLLM   │        │  SANDBOX (image omniforge-sandbox)                        │
│  alias → modèles │        │  --network none · --cap-drop ALL · no-new-privileges      │
│  Anthropic       │        │  limites mémoire / CPU / PID · utilisateur non privilégié │
│  OpenAI          │        │  seul montage : le workspace du run (dépôt git)           │
│  Ollama / vLLM   │        │  outils : git, pytest, ruff, ast-grep                     │
└──────────────────┘        └───────────────────────────────────────────────────────────┘
```

## 1.2 Composants

| Composant | Rôle | Implémentation du MVP | Cible à l'échelle |
|---|---|---|---|
| **API Gateway** | Point d'entrée unique : commandes REST, flux temps réel, authentification | `axum` ; clé d'API statique comparée en temps constant, liste blanche d'origines (CORS, écritures, WebSocket), upload multipart borné à 25 Mo | Passerelle d'entreprise devant le service (TLS, SSO/OIDC, quotas, WAF) ; le service garde sa clé comme défense en profondeur |
| **Orchestration Engine** | Machine à états du SDLC, cycle de vie des runs, détention des verdicts | Tâche `tokio` par run, pipeline explicite (`pipeline.rs`), boucle TDD (`tdd.rs`) | Workers distribués consommant une file ; reprise d'un run à partir de son journal |
| **GateKeeper** | Applique la matrice de supervision à chaque point de contrôle | Fonction pure `evaluate` + attente sur canal `oneshot` (`governance/mod.rs`) | Notifications (chat, e-mail), délégations, quorum |
| **Agent Runtime** | Exécute n'importe quel rôle dans la boucle Observe-Think-Act | `agents.rs` : rôles = données, un seul moteur | Idem ; ajout de rôles par configuration |
| **LLM Router** | Choisit le modèle selon la nature du travail, le budget et les échecs | `llm/router.rs` : table nature → palier → alias, escalade, rétrogradation, repli | Routage appris (coût/qualité observés), cache sémantique |
| **Proxy multi-modèles** | Agnosticité fournisseur, clés, coûts réels | LiteLLM (`config/litellm.config.yaml`), appelé en HTTP compatible OpenAI | Idem + clés virtuelles par projet, budgets, équilibrage de charge |
| **Sandbox Manager** | Isole l'exécution des actions des agents | `sandbox/docker.rs` (conteneur durci par run) ; `sandbox/local.rs` sans isolation pour démo et tests | gVisor / Kata / Firecracker (champ `runtime`), ou runtime distant : seul ce module change |
| **Event Store** | Journal immuable, source de vérité, diffusion en direct | SQLite (WAL) + index mémoire + canal de diffusion par run (`events.rs`) | Postgres pour l'historique, NATS/Kafka pour la diffusion |
| **Vector / Memory Store** | Mémorise leçons et décisions, les rappelle par similarité | Vecteurs de hachage en mémoire (`memory.rs`), interface `Embedder` | Embeddings via LiteLLM + Qdrant ou pgvector, mémoire inter-projets |
| **Sécurité** | Règles déterministes indépendantes des modèles | Scanner statique à règles, garde-fou de commandes, verrouillage des tests, périmètres d'écriture | Intégration Semgrep/Bandit/Trivy, politiques OPA |

## 1.3 Principes d'architecture

**Flux d'événements (inspiré d'OpenHands).** Chaque fait — pensée d'un agent, appel
d'outil, résultat, verdict de test, décision humaine, appel LLM avec son coût — est un
événement numéroté, ajouté à un journal par run. L'état d'un run (`RunState`) est le
résultat d'un réducteur pur appliqué à ce journal (`orchestrator/state.rs`). Conséquences :
piste d'audit complète, rejeu à l'identique après redémarrage (testé), diffusion en direct
sans logique supplémentaire, et une IHM qui ne peut pas diverger du backend puisqu'elle
applique le même réducteur.

**Le workflow est déterministe, les agents sont des exécutants.** Les patterns agentiques
décrits par Anthropic sont utilisés là où ils apportent quelque chose, et le reste est du
code ordinaire :

| Pattern | Où il est appliqué |
|---|---|
| *Prompt chaining* | Ingestion → plan → architecture → tests → code → audit → livraison : chaque étape consomme la sortie validée de la précédente |
| *Routing* | Le classifieur d'entrée choisit le workflow de planification (niveau 1, 2 ou 3) ; le routeur LLM choisit le modèle selon la nature de la tâche |
| *Parallelization* | L'auditeur sécurité et le relecteur de code, en lecture seule, examinent le même changement en parallèle |
| *Orchestrator-workers* | L'orchestrateur distribue les tâches du plan aux agents QA et développeurs, dans l'ordre des dépendances |
| *Evaluator-optimizer* | La boucle TDD : les tests (évaluateur exécutable) et un modèle critique jugent, le développeur corrige |

**Les verdicts appartiennent à l'orchestrateur.** Un agent peut annoncer « les tests
passent » ; cela n'a aucune valeur. L'orchestrateur relance lui-même la suite dans la
sandbox après chaque intervention, exécute le lint, lance le scanner de sécurité, crée les
commits. Les agents ne peuvent ni committer, ni pousser, ni modifier un test verrouillé.

**Moindre privilège.** Chaque rôle a sa liste d'outils et son périmètre d'écriture :
l'architecte n'écrit que dans `docs/`, QA uniquement des fichiers de test, l'auditeur et
le relecteur rien du tout, et seuls les développeurs disposent d'un shell.

**Agnosticité.** Le backend ne référence aucun fournisseur : il manipule des *alias*
(`omniforge-reasoning`, `omniforge-coder`…) résolus par le proxy LiteLLM. Remplacer un
modèle, passer en local ou ajouter un fournisseur est une modification de YAML.

## 1.4 Routage multi-modèles

```text
nature de la tâche     palier        alias (préféré, puis repli)            modèle par défaut
─────────────────────  ───────────   ────────────────────────────────────   ───────────────────────────
planning               reasoning     omniforge-reasoning → omniforge-coder  Claude Opus 5.5
architecture           reasoning
complex_code           reasoning
code                   coder         omniforge-coder → omniforge-reasoning  Claude Sonnet 5.5
test_generation        critic        omniforge-critic → omniforge-coder     GPT-6.1 Sol (OpenAI)
critique               critic
security_review        critic
lint_fix               routine       omniforge-local → omniforge-fast       Llama 3.1 (Ollama) → Claude Haiku 5.5
classification         routine
summarization          routine
```

Trois mécanismes dynamiques complètent la table (`llm/router.rs`) :

- **Escalade.** Après `escalate_after_failures` tentatives GREEN ratées (2 par défaut), le
  développeur monte d'un cran sur l'échelle `routine → coder → reasoning`.
- **Pression budgétaire.** Sous 15 % de budget restant, les appels descendent d'un cran ;
  budget épuisé, l'appel est refusé et devient une exception de supervision.
- **Repli.** Si un modèle échoue, l'alias suivant du palier prend le relais ; chaque
  tentative est tracée (`llm.call`, avec le motif du routage).

Le palier `critic` est volontairement hors de l'échelle : l'évaluateur reste chez un
fournisseur différent de celui du codeur, pour ne pas partager ses angles morts.

Les coûts affichés sont ceux que LiteLLM renvoie pour chaque appel
(`x-litellm-response-cost`). À défaut, une table de prix de repli est appliquée et
l'événement est marqué `cost_estimated`.

> Les modèles cités en exemple dans la demande initiale (Claude 3.5 Sonnet, GPT-4o,
> Llama 3) sont d'anciennes générations ; les alias pointent vers leurs successeurs. Les
> identifiants ont été vérifiés dans la table de prix de LiteLLM 1.108, pas par des appels
> réels : voir §1.8.

## 1.5 Sécurité

| Menace | Parade |
|---|---|
| Code généré malveillant ou défaillant | Exécution dans un conteneur par run, sans réseau, sans capacité, sans élévation possible, mémoire/CPU/PID bornés, délai par commande |
| Évasion par les chemins | Accès fichiers côté hôte via `Workspace` : refus de `..`, des chemins absolus, des liens symboliques sortants et de `.git/` |
| Commandes dangereuses | Garde-fou appliqué avant exécution, aux agents comme à l'humain du terminal (`rm -rf /`, `curl … \| sh`, `sudo`, `git push`, `docker`…) ; violation tracée et remontée en exception |
| Agent qui « triche » sur les tests | Tests verrouillés après RED, refus d'écriture, restauration systématique et violation tracée s'ils ont été modifiés par le shell |
| Fuite de secrets | Environnement vidé pour les commandes des agents ; clés fournisseurs détenues par le seul proxy ; scanner de secrets sur le code produit |
| Git piégé | Git ne s'exécute jamais côté hôte sur un dépôt contrôlé par un agent : uniquement dans la sandbox. L'outil `git` des rôles en lecture seule passe chaque argument tel quel, sans interprétation par le shell, et refuse les options qui écrivent des fichiers |
| Page web malveillante visant une factory locale | Les écritures et les connexions WebSocket portant une origine hors liste blanche sont refusées — CORS seul ne protège ni l'upload multipart ni les WebSockets |
| Saturation | Nombre de runs simultanés plafonné (4 par défaut), lectures de fichiers et diffs bornés |
| Dérive de coût | Budget par run, rétrogradation automatique, portail budgétaire |

**Limite assumée du déploiement Docker.** L'API pilote le démon Docker de l'hôte par son
socket, ce qui équivaut à un accès root sur cet hôte. C'est acceptable sur un poste ou un
hôte dédié aux sandboxes. Pour exécuter du code non fiable à l'échelle, il faut un runtime
à isolation matérielle (Kata, Firecracker) ou un service d'exécution distant ; la frontière
`trait Sandbox` est faite pour cela.

## 1.6 Déploiement

```text
docker compose up
├─ sandbox-image   construit omniforge-sandbox:latest, puis s'arrête
├─ api             backend Rust (utilisateur 10001) ─ volume omniforge-data:/data
│                  └─ lance un conteneur sandbox par run, monté sur le SEUL
│                     sous-répertoire runs/<id>/workspace du volume
├─ web             IHM Next.js
├─ litellm         (profil « llm »)          proxy multi-modèles
└─ ollama          (profil « local-models ») modèles open-source locaux
```

Deux fichiers de configuration, deux responsabilités : `config/omniforge.toml` (routage,
supervision par défaut, limites de sandbox) et `config/litellm.config.yaml` (alias →
modèles, clés).

L'interface peut aussi être déployée seule sur un hébergement statique tel que Vercel : sans
backend, elle rejoue des enregistrements de runs. Les variantes de déploiement sont décrites
dans [5. Déploiement](05-deploiement.md).

## 1.7 Décisions structurantes

| Décision | Alternative écartée | Raison |
|---|---|---|
| Backend en **Rust** (axum, tokio) | Python/FastAPI, TypeScript | Demandé en cours de conception. Un binaire sans dépendance d'exécution, concurrence sûre pour des runs parallèles, typage strict des événements |
| Machine à états explicite + journal d'événements | LangGraph | Le workflow SDLC est connu et fixe : un graphe déclaratif n'apporte rien, et le journal fournit déjà persistance, audit et rejeu. LangGraph est par ailleurs lié à l'écosystème Python/JS |
| LiteLLM en **proxy** | SDK LiteLLM embarqué | Seule option depuis Rust, et la meilleure : clés hors du backend, coûts réels, changement de fournisseur à chaud |
| Tests exécutés par l'orchestrateur | Confiance dans le compte rendu des agents | Un verdict doit être vérifiable ; c'est ce qui rend le mode autonome défendable |
| Scanner de sécurité à règles pour les décisions bloquantes | Jugement du seul modèle auditeur | Reproductible et explicable ; l'auditeur LLM complète, il ne décide pas seul |
| Backend LLM simulé livré avec le produit | Mocks uniquement dans les tests | Démo et intégration continue sans clé, IHM pédagogique utilisable hors-ligne |

## 1.8 Périmètre réel du MVP

Ce qui est en place et vérifié par des tests automatisés : le pipeline complet, la boucle
TDD avec auto-correction, la matrice de supervision et ses portails, le routage avec
escalade, budget et repli, la sandbox Docker (isolation contrôlée sur un vrai démon),
l'ingestion Markdown/texte/PDF, la planification HTN, l'API et le flux temps réel.

Ce qui ne l'est pas, et qu'il faut connaître avant de s'appuyer dessus :

| Limite | Détail |
|---|---|
| Jamais exécuté contre de vrais modèles | Aucune clé fournisseur n'était disponible pendant le développement. Le chemin réel est couvert par un faux serveur compatible OpenAI et par un vrai proxy LiteLLM en mode simulé (format des requêtes, appels d'outils, coûts) ; la qualité des prompts face à de vrais modèles reste à valider |
| Backend simulé | Seul le scénario JWT produit du vrai code ; les autres tâches génèrent un gabarit de démonstration |
| Tâches séquentielles | Les vagues parallélisables sont calculées et affichées, mais les tâches s'exécutent une par une (un workspace partagé). La parallélisation réelle demande un worktree git par tâche |
| Livraison locale | La « merge request » est une branche dans le dépôt du run et un artefact (titre, description, diff). Aucune poussée vers une forge, aucune création de PR distante |
| Projets neufs uniquement | Le workspace part d'un dépôt vide ; le clonage d'un dépôt existant n'est pas implémenté |
| Chaîne de test Python | Les commandes de test et de lint sont globales au run (pytest, ruff par défaut). Le rôle développeur frontend existe, mais des commandes par composant restent à ajouter |
| Reprise après arrêt | L'historique est restauré et un run interrompu est signalé comme tel ; il n'est pas repris |
| Mémoire | En mémoire vive, vecteurs de hachage : rappel de quasi-doublons, pas de recherche sémantique fine |
| Recherche web | L'outil existe dans le contrat des agents mais aucun fournisseur n'est branché : il répond « désactivé » |
| Authentification | Une clé statique partagée : pas d'utilisateurs, de rôles ni de cloisonnement entre projets |
