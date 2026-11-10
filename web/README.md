# OmniForge AI — Mission Control

Interface de pilotage de la factory : graphe multi-agents en direct, volet pédagogique,
terminal et diffs, portail de validation, sélecteur de supervision.

Next.js (App Router, TypeScript) · Tailwind CSS · composants de style shadcn/ui sur Radix ·
React Flow. La conception est décrite dans [`docs/04-ihm.md`](../docs/04-ihm.md).

## Développement

```bash
npm install
npm run dev        # http://localhost:3000 — attend le backend sur http://localhost:8080
```

```bash
npm run lint
npm run build
```

## Configuration

| Variable | Défaut | Rôle |
|---|---|---|
| `NEXT_PUBLIC_API_URL` | voir ci-dessous | Adresse du backend, vue depuis le navigateur |
| `NEXT_PUBLIC_API_KEY` | — | Clé envoyée au backend si celui-ci en exige une |
| `NEXT_PUBLIC_DEMO_MODE` | — | `1` force le mode démonstration |

Ces valeurs sont intégrées au bundle à la construction : les modifier demande un nouveau build.

Sans `NEXT_PUBLIC_API_URL`, l'interface interroge `http://localhost:8080` lorsqu'elle est
servie depuis `localhost`, et passe en **mode démonstration** partout ailleurs.

## Mode démonstration

Sans backend, l'interface rejoue des **enregistrements de vrais runs** (`public/demo/`) à
travers le même réducteur que le flux temps réel. Les portails de validation attendent un
clic, puis la décision enregistrée se déroule. Lancer un run, exécuter une commande ou
changer la supervision demande un backend.

Les enregistrements se régénèrent à partir d'un backend en marche :

```bash
node scripts/record-demo.mjs --api http://127.0.0.1:8080
```

## Déploiement

- **Vercel** : importer le dépôt, régler *Root Directory* sur `web`, déployer. Sans variable
  d'environnement, le site fonctionne en mode démonstration. Détails dans
  [`docs/05-deploiement.md`](../docs/05-deploiement.md).
- **Docker** : `docker build -t omniforge-web --build-arg NEXT_PUBLIC_API_URL=… .`
