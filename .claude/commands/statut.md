---
description: Met à jour le .status.yml du dépôt à partir de son état réel
allowed-tools: Bash(git tag:*), Bash(git log:*), Bash(gh issue list:*), Bash(gh api:*), Read, Edit, Write
---

Rafraîchis le fichier `.status.yml` de ce dépôt.

## Marche à suivre

1. Lis le `.status.yml` existant s'il y en a un. S'il n'y en a pas, crée-le.
2. Relève l'état réel du dépôt :
   - `git tag --sort=-v:refname | head -5` — la version courante
   - `git log -1 --format='%cr — %s'` — la dernière activité
   - `gh issue list --state open --limit 20` — ce qui reste ouvert
   - `gh api repos/{owner}/{repo}/milestones?state=open` — les jalons éventuels
3. Confronte-le à la déclaration : la `phase` est-elle encore juste ? le `jalon`
   est-il atteint ? des tâches de `restantes` sont-elles faites ?
4. Mets `maj` à la date du jour.
5. **Montre le diff et attends l'accord de l'utilisateur avant d'écrire.**
   Ne commite jamais toi-même.

Si une information manque et n'est pas déductible — typiquement le `canal` d'un
projet passé en `publié` — demande-la plutôt que de la deviner.

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
