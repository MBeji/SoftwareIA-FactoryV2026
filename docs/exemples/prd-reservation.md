# Plateforme de réservation de salles

Les collaborateurs réservent des salles de réunion depuis le web. Ce document décrit le
périmètre de la première version.

## Authentification

Gestion des comptes et des sessions.

- Inscription par e-mail
  - Le mot de passe est haché avant stockage
  - Un e-mail déjà utilisé est refusé
- Connexion avec jeton JWT
  - Un jeton expiré est refusé

## Réservations

- Modèle de données des salles
- Création d'une réservation par un utilisateur
  - Deux réservations ne peuvent pas se chevaucher sur la même salle
- Tableau de bord des réservations

## Paiement

- Paiement en ligne avec Stripe
- Webhooks de confirmation de paiement
