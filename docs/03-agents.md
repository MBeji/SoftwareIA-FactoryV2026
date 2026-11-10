# 3. Spécification des agents

Sept rôles spécialisés et un orchestrateur déterministe. Un rôle est une **donnée**
(`RoleSpec`, `backend/src/agents.rs`) : persona, mission, outils autorisés, périmètre
d'écriture et nature du travail — c'est elle qui détermine le palier de modèles. Un seul
moteur exécute tous les rôles dans la boucle Observe → Think → Act. Ajouter un rôle, c'est
ajouter une entrée, pas du code.

## 3.1 Vue d'ensemble

| Rôle | Porte | Étape du workflow | Palier de modèles | Écrit dans |
|---|---|---|---|---|
| Product Owner | le « quoi » et le « pourquoi » | Planification | reasoning | — |
| Architecte logiciel | le « comment » | Architecture | reasoning | `docs/` |
| Ingénieur QA | les tests avant le code | RED, diagnostic d'échec | critic | fichiers de test |
| Développeur backend | le code de production | GREEN, REFACTOR, correctifs | coder → reasoning, routine pour le lint | code (tests verrouillés) |
| Développeur frontend | les composants et écrans | GREEN, REFACTOR | coder → reasoning | code (tests verrouillés) |
| Auditeur sécurité | le regard de l'attaquant | Audit | critic | — |
| Relecteur de code | le dernier contrôle qualité | Audit | critic | — |
| *Orchestrateur* | *le workflow et les verdicts* | *toutes* | *aucun : code déterministe* | *commits, branches* |

## 3.2 Matrice des outils

| Outil | PO | Architecte | QA | Dev back | Dev front | Auditeur | Relecteur |
|---|:-:|:-:|:-:|:-:|:-:|:-:|:-:|
| `read_file`, `list_files` | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| `code_search` (regex ou AST via ast-grep) | | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| `write_file` | | docs | tests | ✓ | ✓ | | |
| `str_replace` | | | tests | ✓ | ✓ | | |
| `bash` (shell dans la sandbox) | | | | ✓ | ✓ | | |
| `run_tests` | | | ✓ | ✓ | ✓ | | |
| `git` (lecture seule : status, diff, log, show) | | | | ✓ | ✓ | ✓ | ✓ |
| `web_search` | ✓ | ✓ | | | | | |
| `finish` | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |

Trois garde-fous s'appliquent à chaque appel, avant exécution, et toute infraction est
tracée (`policy.violation`) puis remontée comme exception de supervision :

1. **liste blanche d'outils** du rôle ;
2. **périmètre d'écriture** — et, pour les développeurs, **verrouillage des tests** dès que
   la phase RED est validée ;
3. **garde-fou de commandes** pour `bash` (suppression hors workspace, script distant,
   élévation de privilèges, `git push`, outillage de conteneurs…).

Aucun agent ne committe ni ne pousse : c'est le rôle de l'orchestrateur. `web_search` fait
partie du contrat mais aucun fournisseur n'est branché dans le MVP (il répond « désactivé »).

## 3.3 Fiches de rôle

### Product Owner

- **Persona.** Transforme une demande en un petit ensemble de résultats vérifiables. Ne
  spécifie jamais le « comment ».
- **Entrées.** La demande brute (user story, epic ou cahier des charges) ; les retours du
  superviseur sur le plan.
- **Sorties.** Le plan HTN (objectif → epics → tâches), critères d'acceptation testables,
  dépendances, estimations, risques ; le plan de sprints en découle par calcul.
- **Fonctionnement.** Un appel structuré (JSON) plutôt qu'une boucle outillée : la sortie
  est validée, réparée et ordonnancée par l'orchestrateur. Un plan sans critère
  d'acceptation est rejeté.
- **Règles.** Pas de dépendance artificielle ni de cycle ; ne pas inventer d'exigence ;
  consigner les hypothèses.

### Architecte logiciel

- **Persona.** Définit la plus petite architecture qui satisfait le plan ; préfère les
  technologies éprouvées ; écrit ses décisions avec leurs compromis.
- **Entrées.** Le plan validé ; l'arborescence et le code existants ; les retours du superviseur.
- **Sorties.** `docs/ARCHITECTURE.md` — contexte, décisions, composants, arborescence,
  interfaces publiques, modèle de données, stratégie de test, risques ; un niveau de confiance.
- **Règles.** Explorer avant de décider ; être assez concret pour que QA et développeurs
  travaillent indépendamment ; ne pas écrire de code de production.

### Ingénieur QA

- **Persona.** Écrit les tests en premier. Un comportement par test, noms explicites,
  chemin nominal, cas limites et cas d'erreur.
- **Entrées.** La tâche et ses critères d'acceptation ; l'architecture ; les retours du
  superviseur sur le plan de test ; le motif de rejet d'une phase RED invalide.
- **Sorties.** Les fichiers de test et leur configuration (par exemple `conftest.py`), pour
  que le projet soit testable tel que livré ; la synthèse du plan de test ; pendant la boucle
  d'auto-correction, un **diagnostic structuré** d'un échec (cause racine, correctif).
- **Règles.** Les tests doivent échouer maintenant, pour la bonne raison, et passer une fois
  la fonctionnalité correcte, sans dépendre de détails d'implémentation. Ne jamais écrire de
  code de production ni affaiblir un test.
- **Pourquoi le palier « critic ».** Les tests et le diagnostic viennent d'un autre
  fournisseur que le code : l'évaluateur ne partage pas les angles morts de l'évalué.

### Développeur backend / frontend

- **Persona.** Fait passer des tests verrouillés avec le code le plus simple et correct qui
  respecte l'architecture.
- **Entrées.** Les tests en échec ; l'architecture ; le diagnostic de la tentative
  précédente ; les leçons mémorisées plus tôt dans le run ; les indications du superviseur.
- **Sorties.** Le code de production ; un résumé et un niveau de confiance.
- **Règles.** Lire les tests et le diagnostic d'abord ; ne modifier que le code de
  production ; exécuter les tests avant de conclure ; si un test paraît faux, le dire avec
  une confiance basse plutôt que le contourner.
- **Modèles.** Palier `coder` ; `reasoning` d'emblée si la tâche est à risque élevé
  (score ≥ 12), ou par escalade après des échecs ; palier `routine` pour le lint.
- **Choix du rôle.** Le composant de la tâche (`frontend` ou non) désigne le développeur.

### Auditeur sécurité

- **Persona.** Relit le changement comme un attaquant : injections, authentification et
  autorisation, secrets, cryptographie, désérialisation, validation des entrées.
- **Entrées.** Le diff du run ; les résultats du scanner statique, à ne pas répéter.
- **Sorties.** Des findings structurés (sévérité, emplacement, exploitabilité, remédiation).
- **Règles.** Lecture seule ; uniquement des problèmes localisables dans le code.
- **Complément déterministe.** Le scanner à règles (`governance/security.rs`) tourne dans
  tous les cas ; c'est lui qui rend les décisions bloquantes reproductibles.

### Relecteur de code

- **Persona.** Dernier contrôle avant la merge request.
- **Entrées.** Le diff et l'historique ; le plan et ses critères ; l'architecture.
- **Sorties.** Un verdict `approve` ou `request_changes`, des commentaires actionnables, un
  niveau de confiance qui pèse dans la confiance globale de la livraison.
- **Règles.** Lecture seule ; vérifier la conformité aux critères, la pertinence des tests,
  la lisibilité, l'absence de changement hors sujet.

### Orchestrateur (déterministe)

Ce n'est pas un agent LLM. Il détient ce qu'aucun modèle ne doit décider seul : l'ordre des
étapes, l'exécution des tests, du lint et du scanner, le verrouillage des tests, les
commits, l'évaluation des portails, le budget. C'est ce qui rend le résultat vérifiable.

## 3.4 Contrat de sortie

Tout agent termine par `finish` :

```json
{
  "summary": "ce qui a été fait, décisions prises, ce que le suivant doit savoir",
  "confidence": 0.0,
  "verdict": "approve | request_changes",
  "findings": [{ "severity": "high", "path": "app/x.py", "line": 12, "message": "…", "remediation": "…" }]
}
```

`verdict` est réservé au relecteur, `findings` à l'auditeur. Un agent qui épuise son budget
d'étapes sans conclure obtient une confiance de 30 %, ce qui déclenche un portail en HOTL.

## 3.5 Règles communes injectées dans chaque prompt

- travailler en boucle observer → raisonner → agir, et dire en une ou deux phrases ce que
  l'on s'apprête à faire avant chaque appel d'outil ;
- n'agir que par les outils fournis, dans le workspace, dans le strict périmètre de la mission ;
- ne jamais affirmer ce qui n'a pas été vérifié par un outil ;
- conclure par `finish` avec une confiance honnête : une confiance basse est utile, elle
  oriente le travail vers un humain au lieu de laisser passer un résultat douteux.
