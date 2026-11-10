export interface Preset {
  id: string;
  level: 1 | 2 | 3;
  label: string;
  hint: string;
  input: string;
}

/** The three reference inputs of the ingestion engine, one per granularity level. */
export const PRESETS: Preset[] = [
  {
    id: "micro",
    level: 1,
    label: "Micro-tâche",
    hint: "User story simple",
    input: "Créer un endpoint FastAPI sécurisé avec JWT",
  },
  {
    id: "epic",
    level: 2,
    label: "Epic",
    hint: "Périmètre fonctionnel",
    input: "Implémenter le module de facturation avec Stripe, webhooks et dashboard",
  },
  {
    id: "prd",
    level: 3,
    label: "Cahier des charges",
    hint: "PRD Markdown",
    input: `# Plateforme de réservation de salles

Les équipes réservent des salles de réunion et règlent les réservations payantes en ligne.

## Authentification
- Inscription par e-mail
  - Le mot de passe est haché
  - Un e-mail en double est refusé
- Connexion avec jeton JWT

## Réservations
- Modèle de données des salles
- Création d'une réservation par un utilisateur
- Tableau de bord des réservations

## Paiement
- Paiement en ligne avec Stripe
- Webhooks de confirmation
`,
  },
];
