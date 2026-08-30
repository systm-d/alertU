---
name: statut-projet
description: Use when work in this repository changes its progress - a feature is finished, a version is tagged or released, a milestone is reached, the project becomes publicly available, or work is paused - to keep .status.yml accurate. Also use when asked where this project stands.
---

# Tenir à jour l'avancement du projet

Ce dépôt déclare son avancement dans `.status.yml` à sa racine. Le fichier est
agrégé avec celui des autres projets par `ws projects`, à la racine du workspace.

## Quand intervenir

Mets le fichier à jour quand quelque chose d'observable a changé :

- une tâche de `restantes` vient d'être terminée → la déplacer vers `faites`
- un tag ou une release vient de sortir → vérifier `phase` et `jalon`
- le projet devient obtenable par quelqu'un d'autre → `phase: publié` + `canal`
- le travail s'arrête pour un moment → `phase: pause`

Ne touche pas au fichier pour un changement qui ne modifie aucun de ces champs :
une déclaration qui bouge à chaque commit ne veut plus rien dire.

## Comment

Modifie le champ concerné, mets `maj` à la date du jour, et signale le
changement à l'utilisateur. Ne commite pas de toi-même. Pour un rafraîchissement
complet à partir de l'état réel du dépôt, la commande `/statut` fait le tour.

<!-- DEBUT-SCHEMA -->
### Le fichier `.status.yml`

Il vit à la racine du dépôt et déclare ce qu'aucun signal git ne peut déduire.
Le reste (version, activité, progression du jalon) est dérivé automatiquement
par `ws projects` et n'a pas à figurer ici.

```yaml
phase: publié          # idée|prototype|bêta|publié|maintenance|pause|archivé
canal: https://…       # où le public obtient le projet
jalon: v1.7.0          # prochaine étape
maj: 2026-08-29        # date de cette mise à jour
taches:
  faites:    [Migration du format horaire]
  restantes: [Écran de correspondances, Revue accessibilité]
notes: |
  Ce qui bloque, en texte libre.
```

**Règles à respecter :**

- `phase` doit valoir exactement l'une des sept valeurs listées. En cas de
  doute : `prototype` tant que rien n'est utilisable, `bêta` dès qu'un tag
  `v0.x` existe, `publié` dès que quelqu'un d'autre que toi peut l'obtenir.
- `canal` est **obligatoire si `phase: publié`**. Il dit où le public obtient
  le projet (Play Store, crates.io, page GitHub Pages, npm…). Il est
  **indépendant de la visibilité du dépôt** : un dépôt privé peut très bien
  livrer une application publique, et c'est le seul endroit qui le dit.
- `jalon` nomme la prochaine étape. Si un jalon GitHub porte exactement ce nom,
  ses chiffres réels priment sur `taches` — inutile alors de tenir les listes.
- `maj` doit être la date du jour à chaque modification du fichier. Au-delà de
  90 jours, `ws projects --check` signale la déclaration comme périmée.
- Avancer une tâche = déplacer sa ligne de `restantes` vers `faites`, sans en
  changer le libellé.
- Ne jamais inventer une phase ou un canal : si l'information n'est pas
  vérifiable dans le dépôt ou sur GitHub, demander à l'utilisateur.
<!-- FIN-SCHEMA -->
