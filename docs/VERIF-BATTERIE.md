# Vérification contradictoire du pourcentage de batterie — A1314 ISO (BCM2042)

Date : 2026-10-01. Appareil : Apple Wireless Keyboard A1314 ISO, `04:DB:56:CA:42:EE`, `0005:05AC:0256`,
firmware `0x0050`, 2 piles AA posées vers 03:00 (clavier hors ligne 02:55 → 03:06).
Question du gérant : le 98-99 % affiché quelques heures après le changement est-il exact ?
Démarche : essayer de **réfuter** ce chiffre.

Niveaux de preuve : **[mesuré]** (matériel ou journal de cette machine), **[source]** (référence citée),
**[modèle]** (calcul sur une courbe publiée, approximative), **[hypothèse]**.

Lecture seule stricte : `HIDIOCGFEATURE` (GET_REPORT) uniquement, nœud en `O_RDONLY`, aucun SET_REPORT,
aucune écriture, pas de sudo, aucun service touché. Le rapport d'appairage n'est ni lu ni cité.

## 0. Verdict

| Question | Réponse | Marge |
|---|---|---|
| Le 98-99 % du matin était-il trop haut ? | **Non, réfutation échouée.** Par bilan de charge, des piles posées il y a 9 h ont consommé 9 à 45 mAh sur 2 000 à 2 850 mAh, soit **98,2 à 99,6 % réels** [modèle]. Le chiffre était juste sur le fond. | réel 97-100 % |
| Et le 96 % de 12:29 ? | **Trop bas.** `0x47` ne baisse que par marches, lors des reconnexions (§1.2bis). Il a perdu 3 points entre 11:00 et 12:29 pendant des perturbations de liaison, sans consommation correspondante. `0xEA` reste à 98, et la table appliquée à `0x49` donne 98,9. | −3 points dus à la liaison |
| Le chiffre vient-il d'une mesure fraîche ? | **Oui.** `power_supply/capacity` n'est pas un cache : chaque lecture provoque un GET_REPORT radio de `0x47` (code noyau + btmon). UPower l'interroge toutes les 30 s. Aucune période noyau de 5 s. | — |
| Que mesure-t-il vraiment ? | Une **tension** (`0x49` = 2 935-2 953 mV, soit ~1,47 V par pile), prise semble-t-il à la reconnexion, projetée sur la table d'usine `0x5A`. Au-dessus de 2 954 mV le firmware affiche 100 %, donc le haut de l'échelle ne dit presque rien. | 1 point = 17,9 mV par paire |
| Restera-t-il juste ? | **Non : il devient optimiste.** La table place 75 % à 1,253 V par pile, ce qui correspond à **~35 % réels** pour une alcaline ; 50 % → ~25 % ; 25 % → ~5 % [modèle]. | ± 10 points |
| La chimie est-elle compatible ? | 2,97-2,99 V quelques heures après la pose : **alcaline** (ou saline). Des NiMH chargées liraient 2,70-2,80 V ; du lithium neuf lirait 3,3-3,5 V. | — |
| Autonomie ? | La pente du premier jour n'est **pas** exploitable (relaxation des piles neuves, radio maintenue active par les lectures). Le jeu précédent affichait encore **90 % firmware après ~180 jours**, soit 62-65 % réels environ. | voir §4 |

**À montrer à l'utilisateur** : le pourcentage firmware, étiqueté « indication du clavier » ; une estimation
propre selon la chimie, avec sa fourchette ; une autonomie en jours, calculée seulement sur l'historique
long d'un jeu de piles ; un avertissement si la tension à la pose ne correspond pas à la chimie déclarée.
Voir §6.

## 1. Chaîne de mesure

### 1.1 D'où vient `0x47`

```
pile ×2 ─► ADC du BCM2042 (pas ≈ 4,4-4,5 mV) ─► 0x46 / 0xFF  tension instantanée (bruit ±1 pas)
                                              └► 0x49        tension « lente » (sans bruit, ~36 mV sous 0x46)
                       table d'usine 0x5A = 0x60 = 0xEB : 2954 / 2506 / 2404 / 2054 mV  (100/75/50/25 %)
                                              └► 0xEA, 0x47  pourcentages entiers (0x47 = Battery Strength,
                                                              lu par le noyau en Feature, quirk PERCENT|FEATURE)
```

* **[mesuré]** `0x46` (u16 LE) = `0xFF[1..3]` (u16 BE), en mV, à un pas d'ADC près. Les valeurs observées
  (2974, 2978, 2982, 2986, 2991) sont espacées de 4 à 5 mV : c'est la quantification de l'ADC.
* **[mesuré]** `0x49` ne bruite jamais : 2953 (03:57), 2950 (04:15-05:15, 13 lectures), 2945 (11:37-12:02,
  11 lectures btmon et le balayage de 12:02). Il reste **36-38 mV sous la moyenne de `0x46`** à 1 h comme
  à 8 h d'écart : ce n'est donc pas un simple filtre passe-bas de `0x46`, qui aurait convergé.
  **[hypothèse]** Tension sous charge (échantillonnée pendant une émission radio) ou valeur minimale tenue.
* **[mesuré]** La table `0x5A` diffère du défaut du code (2900/2450/2350/2000) de +54/+56/+54/+54 mV.
  Ce décalage quasi constant ressemble à un **étalonnage d'ADC propre à l'unité**, appliqué aux seuils
  plutôt qu'à la mesure [hypothèse]. Conséquence : la tension absolue en volts a une incertitude de
  l'ordre de ± 50 mV par paire, mais la comparaison aux seuils, elle, est juste.

### 1.2 Reconstruction de la fonction tension → %

Fonction testée : `pct = tronc(interp_linéaire(0x49 ; 0x5A ↔ 100/75/50/25))`, plafonnée à 100
(`tests/live/re/verif_battery_model.py`, `fw_pct`).

| Heure | `0x49` mV | % calculé (exact) | `0x47` | `0xEA` | noyau / UPower | Accord |
|---|---|---|---|---|---|---|
| 03:07-03:26 | non lu | — | 100 (historique) | — | 100 | — |
| 03:57 | 2953 | 99,94 | 99 | 98 | 99 | ✅ 0x47 |
| 04:15-05:15 | 2950 | 99,78 | 99 | 98 (un 0 à 04:40) | 99 | ✅ 0x47 |
| 11:37-12:02 | 2945 | **99,50** | **98** (137 lectures btmon) | 98 | 98 | ❌ attendu 99 |
| 12:30:49 (après reconnexion 12:29) | 2935 | **98,94** | **96** | 98 | 96 (UPower) | ❌ attendu 98 |

* **[mesuré] Réfutation partielle** : l'hypothèse de `HARDWARE-RAPPORTS-HID.md` §4 (« `0x47` = troncature
  de l'interpolation de `0x49` ») donne 99 à 2945 mV, alors que le clavier rend 98 depuis au moins 25 min.
  Ni la troncature ni l'arrondi (99,497 → 99) ne l'expliquent.
* **[hypothèse]** `0xEA` serait l'estimation brute et `0x47` sa valeur affichée avec retard ou hystérésis :
  `0xEA` vaut 98 dès 03:57 et `0x47` le rejoint entre 10:52 et 11:33 (historique du démon). Autre piste :
  le firmware calcule sur une grandeur interne qu'il n'expose pas. Aucune lecture seule ne peut trancher.
* **Portée pratique : ± 1 point.** La forme de la fonction (plafond à 100 au-dessus de 2954 mV, pente de
  17,9 mV par point jusqu'à 75 %) est solide. Le critère « `floor(pct_fin)` = `0x47` ± 1 » de #139 reste
  satisfait.

### 1.2bis `0x47` ne descend qu'aux reconnexions [mesuré : corrélation ; mécanisme : hypothèse]

Chaque marche de `0x47` suit une déconnexion ou reconnexion du clavier (journal BlueZ et démon) :

| Événement de liaison | `0x47` avant → après | `0x49` à ce moment | % de la table |
|---|---|---|---|
| déconnexion 03:25:47, reconnexion avant 03:41 | 100 → **99** (03:41:47) | ~2953 | 99,9 |
| déconnexion 04:00:33, reconnexion avant 04:15 | 99 → 99 | 2950 | 99,8 |
| redémarrages de bluetoothd 11:15-11:16, `Host is down` 11:32:29 | 99 → **98** (11:33:02) | 2945 | 99,5 |
| déconnexion 12:13:28, reconnexion 12:29:31 | 98 → **96** (12:29:31) | 2935 | 98,9 |

Pendant une session continue, `0x47` reste fixe : 99 de 04:15 à 10:52 alors que `0x49` passe de 2950 à
~2946, puis 98 pendant 137 lectures de 11:37 à 12:02. **[hypothèse]** Le firmware calcule `0x47` au
moment de la (re)connexion, sur une tension prise sous la charge du *paging* radio, et ne le recalcule pas
pendant la session. 96 % correspond à ~2882-2900 mV sur la table, soit 35 à 50 mV sous `0x49` : c'est
l'ordre de grandeur de la chute ohmique d'un pic d'émission.

Conséquences :
* **le pourcentage du noyau ne baisse pas avec la consommation, mais par marches lors des reconnexions.**
  La perte de 3 points entre 11:00 et 12:29 vient des perturbations de liaison du jour (redémarrages de
  BlueZ, lectures massives d'autres agents, décrochage de 12:13). Elle ne vient pas d'une consommation :
  45 mAh en 1,5 h demanderaient 30 mA en continu ;
* sur cette journée, `0x47` est donc plutôt **pessimiste** (96) ; `0xEA` reste à 98 et la table appliquée
  à `0x49` donne 98,9 ;
* la marge de `0x47` est d'au moins −3 points à cause des événements de liaison, en plus de la
  non-linéarité de l'échelle (§2).

### 1.3 Fraîcheur : cache ou mesure ?

* **[source]** `hid-input.c` (`hidinput_get_battery_property`) : si l'état n'est pas `HID_BATTERY_REPORTED`
  et que `avoid_query` est faux, **chaque lecture de `capacity` émet un GET_REPORT** ; sinon le noyau rend
  la dernière valeur reçue par rapport d'entrée. La seule temporisation du code est la limite de 30 s
  de `hidinput_update_battery`, qui ne s'applique qu'aux rapports d'entrée spontanés.
* **[mesuré]** Capture btmon de 11:37 à 12:02 (`re-link/stats1.json`, agent voisin) : 137 réponses
  `DATA type=3 (Feature) id=0x47` et **aucun rapport d'entrée `0x47`** sur le canal d'interruption. L'état
  reste donc « interrogé », jamais « reçu » : `capacity` est **toujours une mesure radio fraîche**.
* **[mesuré]** Intervalles entre requêtes `0x47` : groupes de 2 ou 3 requêtes espacés de **30 s**
  (43 intervalles ≈ 30 s, 81 intervalles de 0 à 1 s) : c'est la scrutation UPower (#146), pas le noyau.
* La « cadence de 5 s » est celle du `TICK` du démon (`actor.rs:119`), qui republie un instantané et ne
  relit pas le clavier toutes les 5 s. Le démon fait une lecture complète toutes les 15 min
  (« acquired … » dans le journal : 11:33, 11:48, 12:03).
* **[hypothèse] Artefact de mesure** : ce matin, la radio a été tenue éveillée par environ 32 GET_REPORT
  par minute (812 en 25 min, plusieurs agents d'étude en plus d'UPower). La consommation du jour et la
  pente de tension du jour ne sont donc **pas représentatives** d'un usage normal.

## 2. Modèle physique

Tension par pile = tension mesurée / 2. Courbes de décharge à faible débit (~1 mA, 21 °C), **approximatives**
(± 0,03 V, ± 10 points) : synthèse des fiches Energizer E91 (alcaline), L91 (Li-FeS₂), Eneloop/NH15 (NiMH) et
[Wikipedia AA battery](https://en.wikipedia.org/wiki/AA_battery). Les fiches E91 et L91 ne donnent leurs courbes
qu'en graphique : les valeurs du tableau sont lues à l'œil, pas copiées. Résistance interne : alcaline
150-300 mΩ, L91 120-240 mΩ [source, fiches]. À quelques mA, la chute ohmique fait moins de 3 mV par paire,
donc la tension lue ≈ tension à vide.

Consommation d'un clavier BT 2.0 [source qualitative, fiche BCM2042 : « autonomie > 6 mois »] :
environ 0,1 mA en veille profonde, 0,5 à 1 mA connecté au repos, quelques mA en frappe. Ordre de grandeur
moyen : 0,3 à 1 mA en usage normal.

Tableau produit par `verif_battery_model.py` (état réel = 100 − profondeur de décharge, à la tension donnée) :

| Paire (mV) | V/pile | % firmware | Alcaline réel | NiMH Eneloop réel | Lithium L91 réel |
|---|---|---|---|---|---|
| 2991 | 1,496 | 100 | ~89 | (impossible au repos) | ~13 |
| 2950 | 1,475 | 99 | ~84 | (impossible au repos) | ~11 |
| 2935 | 1,468 | 98 | ~82 | — | ~10 |
| 2900 | 1,450 | 96 | ~78 | — | ~9 |
| 2775 | 1,388 | 90 | ~62 | ~99 | ~7 |
| 2600 | 1,300 | 80 | ~44 | ~92 | ~4 |
| **2506** | 1,253 | **75** | **~35** | ~63 | ~4 |
| **2404** | 1,202 | **50** | **~25** | ~21 | ~3 |
| 2200 | 1,100 | 35 | ~10 | ~4 | ~2 |
| **2054** | 1,027 | **25** | **~5** | ~1 | ~1 |

Lecture par chimie, pour 2,98-2,99 V (≈ 1,49 V par pile) :

* **Alcaline** : sur la courbe de tension, ~85-90 % réels. Mais **le bilan de charge prévaut sur des piles
  neuves** : 9 h × 1 à 5 mA = 9 à 45 mAh sur ~2 500 mAh, soit plus de 98 % réels. L'écart vient du plateau de
  surface (1,58-1,62 V à neuf), qui disparaît dans les tout premiers pour cent. **98-99 % est plausible.**
  Une alcaline vraiment neuve se lit plutôt vers 1,52-1,55 V par pile sous faible charge : 1,49 V suggère
  soit des piles déjà stockées longtemps, soit des salines (zinc-carbone), soit le décalage d'ADC du §1.1.
* **NiMH (Eneloop)** : 1,49 V par pile est impossible au repos (1,35-1,42 V juste après la charge, plateau
  à 1,20-1,25 V). Avec des NiMH, le firmware afficherait **85-90 % dès la pose**, puis **50-75 % pendant
  presque toute la vie** des piles, et 25 % à la toute fin : le pourcentage serait faux dans l'autre sens.
* **Lithium (L91)** : 1,49 V par pile correspond à la **fin de vie** (moins de 15 % restants). Neuves, elles
  liraient 3,3 à 3,5 V par paire, au-dessus de la plage d'alimentation de 2,7-3,3 V annoncée pour le BCM2042
  (seul un régulateur sur la carte rendrait ce cas sûr, non vérifié). Elles afficheraient 100 % pendant
  ~90 % de leur vie, puis s'effondreraient en quelques jours.

→ **Seule l'alcaline (ou la saline) rend 98-99 % cohérent** avec 2,97-2,99 V quelques heures après la pose.

## 3. Série temporelle (03:07 → 12:15, 9 h, sans lecture supplémentaire du clavier)

La série « 3 h en direct » prévue a été remplacée par les mesures déjà enregistrées, plus longues :

* à 12:13:12, la première salve de `verif_battery_series.py` a reçu un EIO (`hidp_report_req_timeout`)
  et l'outil s'est arrêté tout seul. Le clavier s'est déconnecté à 12:13:28, pendant les lectures
  d'un autre agent ;
* consigne du coordinateur (12:20) : suspension. Reprise seulement si `Connected: yes`, avec
  4 rapports (`0x46`, `0x49`, `0x47`, `0xEA`) une fois par 5 min et arrêt à la première erreur. Les salves
  obtenues après cette heure sont dans `tests/live/re/verif_series_20261001.jsonl` (§3.3).

### 3.1 Pourcentage (historique du démon, `~/.config/apple-kb-monitor/history.jsonl`, et btmon)

| Fenêtre | `0x47` / noyau | Durée |
|---|---|---|
| 03:07:45 → 03:26 | 100 % | ~20-35 min |
| 03:41:47 → 10:52 | 99 % | ~7 h 10 |
| ≤ 11:33:02 → 12:13 (déconnexion) | 98 % | — |

Le jeu d'avril s'est comporté de la même façon : **100 % à 00:43 le 04/04, puis 98 % dès 10:52**,
et 98 % pendant les 2,5 jours suivants. La descente 100 → 98 en moins de 10 h se **reproduit** d'un jeu de
piles à l'autre : c'est la relaxation de la tension des piles neuves, pas une consommation de 2 %
(qui demanderait ~50 mAh en 8 h, soit 6 mA en continu).

### 3.2 Tensions

| Fenêtre | n | `0x46` moyenne (σ) | `0xFF` moyenne | `0x49` | `0x46` − `0x49` |
|---|---|---|---|---|---|
| 03:57 (audit) | 2 | 2991 | 2982-2991 | 2953 | 38 |
| 04:15-05:15 (fixture) | 13 | 2988,3 (2,5) | 2987,5 | 2950 | 38 |
| 11:37-11:51 (btmon) | 11 | 2981,6 (2,7) | 2982,0 | 2945 | 37 |
| 12:02 (balayage voisin) | — | 2974-2982 | 2969-2978 | 2945 | ~33 |

* Quantification : pas d'ADC ≈ 4,4-4,5 mV ; bruit de `0x46` = ±1 pas (σ ≈ 2,5 mV). `0x49` évolue par
  marches de 3 à 5 mV, environ toutes les quelques heures.
* `0x47` évolue par marches d'un point entier : 1 point = 17,9 mV par paire sur le segment 100-75 %.
  À la pente du jour, cela fait **une marche toutes les 20 à 25 h**.
* **Pente** (centres 04:45 → 11:44, 7 h) : `0x46` −0,93 mV/h, `0x49` −0,71 mV/h.

### 3.3 Salves légères après reprise

`verif_series_20261001.jsonl` : 11 salves de 12:30:49 à 13:21:13, une toutes les 5 min, 4 rapports par
salve, sans aucune erreur. Le clavier s'était reconnecté de lui-même à 12:29:31. L'échantillonneur a été
arrêté à la main à 13:21 et aucun processus ne reste actif.

| Heure | `0x46` mV | `0x49` mV | `0x47` | `0xEA` | UPower |
|---|---|---|---|---|---|
| 12:30:49 | **2978** | 2935 | 96 | 98 | 96 % |
| 12:35:52 | 2982 | 2935 | 96 | 98 | 96 % |
| 12:40:55 | 2982 | 2935 | 96 | 98 | 96 % |
| 12:45:57 → 13:21:13 (8 salves) | 2986 | 2935 | 96 | 98 | 96 % |

* `0x46` **remonte** de 2978 à 2986 mV dans les 15 min qui suivent la reconnexion : c'est la récupération
  d'une alcaline après un appel de courant. Cela appuie l'hypothèse d'une tension prise sous charge
  au moment de la reconnexion (§1.2bis).
* `0x49` reste fixe à 2935 et `0x47` à 96 pendant 50 min : aucune remontée de `0x47` dans la session,
  ce qui confirme qu'il est figé jusqu'à la prochaine reconnexion. Pas de pente mesurable en 50 min.
* UPower = `0x47` à chaque salve : aucune valeur périmée.

### 3.4 Estimation d'autonomie à partir de la pente : **non valable**

En prolongeant −0,71 mV/h jusqu'à 2054 mV (25 % firmware, ~5 % réels), on obtient ~1 250 h, soit **~52 jours**.
C'est **faux par construction** :
1. une alcaline perd sa tension bien plus vite au début de sa vie que sur son plateau ;
2. aujourd'hui la radio a été tenue éveillée par plus de 30 requêtes par minute ;
3. le jeu précédent a tenu au moins 180 jours (§4).

Une pente n'a de sens qu'après plusieurs semaines, et seulement sur `0x49` (sans bruit, pas fin).

## 4. Jeu de piles précédent

Fichiers : `~/.local/share/apple-kb-monitor/history.jsonl.bak` (04/04 00:06 → 11:37, 1 264 lignes) et
`history.jsonl` (04/04 11:38 → 01/10 03:22, 872 lignes) ; `~/.config/apple-kb-monitor/history.jsonl`
(= `~/.local/state/…`, lien symbolique ; 01/10 02:33 → 12:03, 46 lignes). Le champ `voltage` est la constante
`0xF5` (2,9777 / 2,9806 / 2,9032 V) : **il n'est pas utilisé**.

| Date | % | Remarque |
|---|---|---|
| 04/04 00:06 | 98 | ancien jeu |
| 04/04 00:43 | 100 | **pose d'un jeu** [hypothèse : saut 98 → 100] |
| 04/04 10:52 → 07/04 | 98 | même relaxation qu'aujourd'hui |
| 07/04 → 01/10 | **aucune donnée** | démon arrêté ou historique non écrit : 177 jours de trou |
| 01/10 02:33 → 02:54 | **90** | dernier état de l'ancien jeu |
| 01/10 03:07 | 100 | pose du jeu actuel |

* **Impossible de calibrer une relation temps-% sur une décharge complète** : il n'y en a pas dans
  l'historique, seulement deux points (98 % le 07/04, 90 % le 01/10) séparés par 177 jours sans mesure.
* Ce qu'on peut en tirer [modèle] : 90 % firmware ⇒ `0x49` entre 2775 et 2793 mV ⇒ 1,388-1,397 V par
  pile ⇒ **environ 62-65 % réels (± 10)** pour une alcaline. **Les piles retirées le 01/10 avaient encore
  probablement plus de la moitié de leur capacité.**
* Si le jeu a bien été posé le 04/04 et utilisé régulièrement, ~35-40 % consommés en 180 jours donnent une
  durée de vie de **15 à 17 mois** (fourchette large, 10 à 24 mois). Cette valeur est incertaine : l'usage
  réel pendant le trou n'est pas connu.
* La fiche BCM2042 annonce « plus de 6 mois », et l'ancien jeu à 90 % au bout de 6 mois s'y accorde :
  **le pourcentage firmware n'a pas été démenti par le jeu précédent**, il est simplement très lent à
  descendre en haut d'échelle.

## 5. Rapports lisibles : rien d'oublié ?

Les 27 IDs de la carte (`HARDWARE-RAPPORTS-HID.md` §2), confrontés à toutes les mesures du jour (balayage,
série 04:15-05:15, btmon 11:37-12:15, balayage voisin de 12:02) :

| Varie avec le temps ou la charge | Exploité ? |
|---|---|
| `0x46` tension instantanée | prévu par #139 (le code lit encore `0xF5`, #136) |
| `0xFF` octets 1-2 = `0x46` | doublon de `0x46` ; octet 3 = `0x01` constant (drapeau à surveiller en fin de vie) |
| `0x49` tension lente | prévu par #139 ; **c'est la grandeur que suit le firmware** |
| `0x47` % | oui (noyau, démon) |
| `0xEA` % (en avance sur `0x47`, 0 transitoire) | non (écrasé par le noyau côté Rust, #136) ; utile comme signe avant-coureur de la marche suivante |
| `0xFE` (`fe0004` une fois, puis des zéros) | non : vraisemblablement un artefact ; **ne plus le lire** (le clavier n'y a pas répondu juste avant le décrochage de 12:13) |

Constants sur 9 h : `0x09`, `0x4A` (18), `0x4B`, `0x4F`, `0x51-0x54`, `0x5A`/`0x60`/`0xEB`, `0x5B`, `0x5C`,
`0x5D`, `0xD1`, `0xD8`, `0xF4`, `0xF5`, `0xF6`, `0xF7`. **Aucun rapport variable n'a été oublié.**
`0x4A` = 18 aurait pu être une température en °C [hypothèse faible], mais il est resté à 18 de 03:57 à
12:02 : à vérifier une seule fois par une journée chaude, sans lecture supplémentaire.

Grandeur dérivée inexploitée : **l'écart `0x46` − `0x49`** (36-38 mV, stable). Si `0x49` est bien une tension
sous charge, cet écart suit la résistance interne, qui augmente quand une alcaline s'use : c'est un
indicateur de santé possible (commentaire sur #108).

## 6. Ce qu'il faut afficher

1. **Pourcentage firmware** (`0x47`, = noyau) : à garder, avec le libellé « indication du clavier ».
   Ne jamais le présenter comme une charge restante linéaire.
2. **Estimation propre** : `0x49` projeté sur la courbe de la chimie déclarée (alcaline par défaut),
   avec sa fourchette (± 10 points). Pendant les 2 premiers jours d'un jeu : « piles neuves », sans chiffre
   tiré de la tension.
3. **Autonomie en jours** : seulement à partir d'un historique d'au moins 2 semaines sur `0x49`, ou de la
   durée de vie mesurée du jeu précédent ; jamais à partir de la pente du premier jour.
4. **Avertissement de chimie** : `0x49` < 2,85 V juste après la pose → NiMH probable, le % firmware sera
   bas toute la vie ; `0x46` > 3,15 V → lithium, le % restera à 100 jusqu'à la fin, puis s'effondrera.
5. **Alertes** : les seuils 30/15/5 % du démon s'appliquent à l'échelle firmware. **30 % firmware ≈ 7 %
   réels** pour une alcaline : la première alerte arrive trop tard (issue dédiée).

## 7. Issues

| # | Objet |
|---|---|
| #178 (bug) | alertes 30/15/5 % sur l'échelle firmware : la 1re alerte arrive vers ~7 % réels |
| #179 (doc/RE) | `0x47` ≠ tronc(interp(`0x49`)) à 2945 mV ; `0x49` ≠ filtre de `0x46` : corriger la carte et #139 |
| #180 (bug) | historique : `voltage` = constante `0xF5` sur 2 136 lignes, à marquer non fiable et à remplacer par `0x46`/`0x49` |
| #83 (commentaire) | ne pas régresser sur le % firmware ni sur la pente des premiers jours |
| #85 (commentaire) | `BatteriesInstalledAt` = 0 après redémarrage ; seuil de 300 mV trop haut (saut réel de +160 à +180 mV) |
| #108 (commentaire) | règles de chimie à la pose, table firmware → réel, écart `0x46` − `0x49` |
| #139 (commentaire) | égalité exacte réfutée ; le % fin reste sur l'échelle firmware |

## 8. Reproduire

```bash
python3 tests/live/re/verif_battery_model.py                      # tableau tension → % par chimie, sans matériel
python3 tests/live/re/verif_battery_model.py --mv 2945 2775       # points précis
python3 tests/live/re/verif_battery_series.py --analyze tests/live/re/verif_series_20261001.jsonl
# mode léger, attend Connected: yes, 4 rapports / 5 min, arrêt à la 1re erreur :
python3 tests/live/re/verif_battery_series.py 04:DB:56:CA:42:EE '' --rounds 36 --period 300 --wait-node --out s.jsonl
```

Sources : [hid-input.c](https://github.com/torvalds/linux/blob/master/drivers/hid/hid-input.c)
(`hidinput_get_battery_property`, `hidinput_query_battery_capacity`, quirk `ALU_WIRELESS_2011_ISO`) ;
[Energizer E91](https://data.energizer.com/pdfs/e91.pdf) ; [Energizer L91](https://data.energizer.com/pdfs/l91.pdf) ;
[Wikipedia — AA battery](https://en.wikipedia.org/wiki/AA_battery) ; fiche BCM2042 (voir `RE-FIRMWARE-MAINTENANCE.md`).
