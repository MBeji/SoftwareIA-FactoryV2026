# 5. Déploiement

Trois façons de faire tourner OmniForge, de la plus simple à la plus complète.

| Objectif | Ce qu'il faut | Section |
|---|---|---|
| Montrer l'interface, sans rien héberger d'autre | Vercel seul | [5.1](#51-vercel-seul--mode-démonstration) |
| Tester la factory de bout en bout depuis Vercel | Vercel + un backend hébergé | [5.2](#52-vercel--un-backend-hébergé) |
| Faire tourner toute la pile sur une machine | Docker Compose | [5.3](#53-docker-compose) |

## Pourquoi le backend ne va pas sur Vercel

Vercel exécute des fonctions courtes et sans état. Le backend OmniForge a besoin de
l'inverse : un processus qui vit aussi longtemps qu'un run, des connexions WebSocket, un
disque (journal d'événements, workspaces git) et un démon Docker pour les sandboxes. Sur
Vercel, on déploie donc l'**interface** ; le backend tourne ailleurs.

## 5.1 Vercel seul — mode démonstration

Aucune variable à définir, aucun backend.

1. Sur [vercel.com/new](https://vercel.com/new), importer le dépôt GitHub.
2. Dans **Root Directory**, choisir `web` (l'application Next.js n'est pas à la racine du
   dépôt ; ce réglage se fait dans l'écran d'import, il ne peut pas venir d'un fichier du dépôt).
3. Laisser le reste tel quel — Vercel détecte Next.js — puis **Deploy**.

Vercel déploie la branche par défaut du dépôt et crée une préversion pour chaque pull request.

Sans adresse de backend, l'interface passe d'elle-même en **mode démonstration** : elle
rejoue trois enregistrements de vrais runs, à travers le même réducteur que le flux temps
réel.

| Enregistrement | Ce qu'il montre |
|---|---|
| Micro-tâche sécurisée — HITL | Les quatre portails de validation, une révision du plan, RED → GREEN → REFACTOR, l'auto-correction, un correctif de sécurité |
| Cahier des charges — autonome | 7 tâches sur 2 sprints sans intervention, un humain sur la seule merge request |
| Divergence TDD — HOTL | L'exception levée après 2 itérations, puis l'arrêt du run par le superviseur |

Ce que le mode démonstration ne fait pas : lancer un nouveau run, exécuter une commande dans
la sandbox, changer la supervision. Les portails attendent bien votre clic, mais c'est la
décision enregistrée qui se déroule ensuite. Un bandeau le rappelle dans l'interface.

Les enregistrements se régénèrent avec `node web/scripts/record-demo.mjs` (backend en marche).

## 5.2 Vercel + un backend hébergé

L'interface sur Vercel, le backend sur une machine ou une plateforme de conteneurs.

### Côté backend

Le backend doit être joignable **en HTTPS** : une page servie en HTTPS ne peut appeler ni
une API en HTTP, ni un WebSocket non chiffré (`wss://` obligatoire).

| Hébergement | Image | Sandbox | Usage |
|---|---|---|---|
| Machine avec Docker (VM, serveur) | `backend/Dockerfile` via `docker compose` | Docker, un conteneur isolé par run | Vrais modèles, usage réel |
| Plateforme de conteneurs sans démon Docker (Render, Railway, Fly.io…) | `backend/Dockerfile.demo` | Locale : les commandes des agents tournent dans le conteneur de l'API, **sans isolation par run** | Essai avec le LLM simulé uniquement |

Variables à définir sur le backend :

| Variable | Valeur |
|---|---|
| `OMNIFORGE_API_KEY` | Une clé longue et aléatoire. Sans elle, n'importe qui peut lancer des runs |
| `OMNIFORGE_CORS_ORIGINS` | L'adresse exacte du site Vercel, par exemple `https://mon-projet.vercel.app`. Plusieurs adresses se séparent par des virgules |
| `OMNIFORGE_BIND` | `0.0.0.0:8080` (l'image de démonstration lit `PORT`) |

Les écritures et le flux temps réel venant d'une origine absente de
`OMNIFORGE_CORS_ORIGINS` sont refusés : chaque adresse de préversion Vercel à utiliser doit
donc y figurer.

### Côté Vercel

Dans **Settings → Environment Variables** du projet, puis redéployer (ces valeurs sont
intégrées au bundle à la construction) :

| Variable | Valeur |
|---|---|
| `NEXT_PUBLIC_API_URL` | L'adresse HTTPS du backend, sans barre oblique finale |
| `NEXT_PUBLIC_API_KEY` | La même valeur que `OMNIFORGE_API_KEY` |

`NEXT_PUBLIC_API_KEY` est lisible dans le code servi au navigateur : elle empêche un
inconnu d'appeler l'API directement, pas un visiteur du site de s'en servir. Pour un usage
au-delà de l'essai, placez l'interface et l'API derrière une authentification (SSO,
passerelle d'API).

### Backend sur son propre poste

Une interface servie par Vercel ne peut pas appeler de façon fiable un backend sur
`localhost` : les navigateurs restreignent les requêtes d'un site public vers le réseau
local. Exposez plutôt le backend local par un tunnel HTTPS (Cloudflare Tunnel, ngrok…) et
donnez l'adresse du tunnel à `NEXT_PUBLIC_API_URL` — ou lancez simplement l'interface en
local avec `npm run dev`.

## 5.3 Docker Compose

Toute la pile sur une machine disposant de Docker.

```bash
cp .env.example .env
echo "DOCKER_GID=$(stat -c '%g' /var/run/docker.sock)" >> .env
docker compose up --build
```

| Service | Rôle | Adresse |
|---|---|---|
| `web` | Interface Mission Control | http://localhost:3000 |
| `api` | Backend ; lance un conteneur sandbox par run | http://localhost:8080 |
| `sandbox-image` | Construit l'image des sandboxes, puis s'arrête | — |
| `litellm` (profil `llm`) | Proxy multi-modèles | réseau interne |
| `ollama` (profil `local-models`) | Modèles open-source locaux | réseau interne |

Pour de vrais modèles : renseigner `ANTHROPIC_API_KEY`, `OPENAI_API_KEY` et
`LITELLM_MASTER_KEY` dans `.env`, passer `OMNIFORGE_LLM_MODE=gateway`, puis
`docker compose --profile llm up --build`.

Les ports ne sont publiés que sur `127.0.0.1`. Avant d'exposer la pile : définir
`OMNIFORGE_API_KEY`, placer un proxy TLS devant, et garder en tête que l'API pilote le démon
Docker de l'hôte (voir [Architecture, §1.5](01-architecture.md#15-sécurité)).

## 5.4 Avant d'exposer un backend

- `OMNIFORGE_API_KEY` définie, `OMNIFORGE_CORS_ORIGINS` limitée aux adresses de l'interface.
- HTTPS devant l'API (WebSocket compris).
- Sandbox Docker dès que de vrais modèles écrivent le code ; l'image de démonstration et
  `--sandbox local` sont réservées au LLM simulé.
- Un budget par run (`budget_usd`) dès que les appels LLM sont facturés.
