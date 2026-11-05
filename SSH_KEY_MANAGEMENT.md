# Schéma de fonctionnement de la gestion des clés SSH

## Contexte

Chaque machine ne peut écrire que dans son propre dossier de configuration. Pour établir une connexion SSH entre deux machines, il faut échanger les clés publiques.

## Acteurs

- **Machine Source (S)** : Machine qui initié la connexion SSH
- **Machine Cible (T)** : Machine qui reçoit la connexion SSH
- **Utilisateur Source (Uₛ)** : Utilisateur sur la machine source
- **Utilisateur Cible (Uₜ)** : Utilisateur sur la machine cible

## Workflow d'installation

```
Machine Source (S)                             Machine Cible (T)
        |                                              |
        |  1. Utilisateur appuie sur 'i' dans la TUI   |
        |  2. Vérification que T est joignable (SSH)     |
        |  3. Vérification qu'aucune connexion existe     |
        |     S→Uₛ→Uₜ@T                                 |
        |  4. Vérification qu'aucune clé existe         |
        |     ~/.ssh/id_regroup_Uₛ@T                   |
        |  5. Génération de la paire de clés             |
        |     ssh-keygen -t ed25519 -N "" -f ...        |
        |  6. Extraction de la clé publique              |
        |                                              |
        +---------------------------------------------->|
        |  ssh Uₜ@T "regroup receive-install-key         |
        |    --source-user Uₛ                          |
        |    --source-machine S                         |
        |    --public-key <pub_key>                     |
        |    --connection-id Uₛ->Uₜ@T"                  |
        |                                              |
        |  7. Vérification des paramètres                |
        |     (user local == Uₜ, machine == T)          |
        |  8. Vérification qu'aucune connexion          |
        |     entrante existe: Uₛ@S->Uₜ.json            |
        |  9. Sauvegarde de la connexion                |
        |     devices/T/connections/Uₛ@S->Uₜ.json       |
        |  10. Ajout de la clé publique dans            |
        |      ~/.ssh/authorized_keys                  |
        |      (avec commentaire regroup:...)          |
        |  11. Commit git local                         |
        |      "Add connection: Uₛ@S→Uₜ"               |
        |                                              |
        |  12. Sauvegarde de la connexion (source)      |
        |      devices/S/connections/Uₛ->Uₜ@T.json     |
        |  13. Commit git local (source)                |
        |      "Add connection: Uₛ@S→Uₜ@T"              |
        v                                              v
```

## Workflow de suppression

```
Machine Source (S)                             Machine Cible (T)
        |                                              |
        |  1. Utilisateur appuie sur 'r' dans la TUI   |
        |  2. Recherche de la première connexion        |
        |     sortante: Uₛ->Uₜ@T.json                   |
        |  3. Extraction de connection_id:              |
        |     Uₛ->Uₜ@T                                  |
        |  4. Vérification que T est joignable (SSH)     |
        |                                              |
        +---------------------------------------------->|
        |  ssh Uₜ@T "regroup receive-remove-key         |
        |    --connection-id Uₛ->Uₜ@T"                 |
        |                                              |
        |  5. Vérification des paramètres                |
        |     (user local == Uₜ, machine == T)          |
        |  6. Recherche de la connexion entrante        |
        |     Uₛ@S->Uₜ.json                            |
        |  7. Suppression de la connexion                |
        |     devices/T/connections/Uₛ@S->Uₜ.json       |
        |  8. Suppression de la clé de                  |
        |      ~/.ssh/authorized_keys                  |
        |  9. Commit git local                         |
        |      "Remove connection: Uₛ@S→Uₜ"             |
        |                                              |
        |  10. Suppression de la connexion (source)     |
        |      devices/S/connections/Uₛ->Uₜ@T.json    |
        |  11. Commit git local (source)                |
        |      "Remove connection: Uₛ@S→Uₜ@T"           |
        v                                              v
```

## Structure des fichiers

### Connexion Sortante (sur Machine Source S)
```
devices/S/connections/Uₛ->Uₜ@T.json
```
Contenu :
```json
{
  "from_user": "Uₛ",
  "from_machine": "S",
  "to_user": "Uₜ",
  "to_machine": "T",
  "public_key": "ssh-ed25519 AAA...",
  "private_key_path": "/home/Uₛ/.ssh/id_regroup_Uₛ@T",
  "created_at": "2024-11-05T12:00:00Z"
}
```

### Connexion Entrante (sur Machine Cible T)
```
devices/T/connections/Uₛ@S->Uₜ.json
```
Contenu :
```json
{
  "from_user": "Uₛ",
  "from_machine": "S",
  "to_user": "Uₜ",
  "to_machine": "T",
  "public_key": "ssh-ed25519 AAA...",
  "private_key_path": "/home/Uₛ/.ssh/id_regroup_Uₛ@T",
  "created_at": "2024-11-05T12:00:00Z"
}
```

### Clé SSH (sur Machine Source S)
```
~/.ssh/id_regroup_Uₛ@T          (clé privée, permissions 0600)
~/.ssh/id_regroup_Uₛ@T.pub     (clé publique, permissions 0644)
```

### Authorized Keys (sur Machine Cible T)
```
~/.ssh/authorized_keys
```
Contient une ligne :
```
ssh-ed25519 AAA... regroup:Uₛ->Uₜ@T
```

## Nomenclature

- **Clé SSH** : `id_regroup_<user>@<target_machine>`
- **Connection ID** : `<from_user>-><to_user>@<to_machine>`
- **Fichier de connexion sortante** : `<from_user>-><to_user>@<to_machine>.json`
- **Fichier de connexion entrante** : `<from_user>@<from_machine>-><to_user>.json`

## Règles

1. Chaque machine ne peut écrire que dans son propre répertoire `devices/<machine>/`
2. Regroup gère une seule clé par connexion (une paire de machines/utilisateurs)
3. Si une clé ou une connexion existe déjà, l'opération échoue avec un message clair
4. Les deux machines (source et cible) committent leurs changements localement
5. L'authentification SSH se fait par mot de passe (l'utilisateur entre le mot de passe demandé par SSH)
6. La TUI se cache temporairement pendant la saisie du mot de passe SSH

## Commandes CLI (internes)

Ces commandes sont appelées via SSH et ne sont pas destinées à être utilisées directement par l'utilisateur.

### receive-install-key
```bash
regroup receive-install-key \
  --source-user <user> \
  --source-machine <machine> \
  --public-key <key> \
  --connection-id <id>
```

### receive-remove-key
```bash
regroup receive-remove-key \
  --connection-id <id>
```

## Commandes TUI

- `i` : Installer une clé SSH vers la première machine connue
- `r` : Supprimer une clé SSH de la première connexion sortante
