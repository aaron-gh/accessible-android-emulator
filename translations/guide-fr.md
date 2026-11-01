# Traduction française d'AAE : guide de style et glossaire

Validé le 26 octobre 2026. Les choix tranchés sont résumés en section 6.

## 1. Principes

- **Pour qui** : des personnes aveugles ou malvoyantes qui testent des apps Android avec VoiceOver, NVDA, JAWS ou le Narrateur. Le texte est d'abord **entendu** : il doit sonner juste à l'oreille, pas seulement à l'écrit.
- **Des mots déjà connus** : on reprend les termes de TalkBack, d'Android, de macOS et de Windows en français. L'utilisateur ne doit jamais apprendre un deuxième nom pour une chose qu'il connaît déjà.
- **Naturel avant littéral** : on traduit le sens, pas les mots. Une phrase française ne doit jamais trahir l'anglais derrière elle.
- **Un terme par notion** : une fois choisi, le terme est utilisé partout, dans l'app Mac, l'app Windows, la ligne de commande et le README.

## 2. Ton et grammaire

- **Vouvoiement**, comme Apple, Google et Microsoft. Pas de « on ».
- **Boutons, menus et commandes à l'infinitif** : « Démarrer », « Enregistrer un instantané… », « Forcer l'arrêt ».
- **Consignes à l'impératif** dans les phrases : « Choisissez Nouvel appareil pour en créer un. »
- **Pas de « Veuillez »** ni de formules de politesse : on va droit au but, comme l'anglais d'AAE.
- **Phrases courtes**, faciles à suivre à l'écoute. On découpe une phrase longue plutôt que de l'alourdir.
- **Noms variables** : le nom d'un appareil peut être n'importe quoi (« Test », « Pixel 9 »). On écrit donc « L'appareil {device} est prêt » plutôt que « {device} est prêt », pour éviter les accords impossibles.
- **Pluriels** : chaque texte avec un nombre a sa forme au singulier et au pluriel (« 1 appareil », « 3 appareils »). En français, 0 prend le singulier (« 0 appareil »).

## 3. Typographie

- **Majuscule seulement au premier mot** et aux noms propres, y compris dans les titres et les menus : « Démarrage à froid », pas « Démarrage À Froid ».
- **Espace insécable** avant « : ; ? ! », avant « % », entre un nombre et son unité, et à l'intérieur des guillemets.
- **Guillemets français** « », pas "…".
- **Points de suspension** : le caractère « … », collé au mot, sur les commandes qui ouvrent une fenêtre (« Renommer… »), comme en anglais.
- **Pas de point final** dans un libellé sans verbe conjugué (bouton, titre, option).
- **Unités** : Ko, Mo, Go, sans « s » au pluriel. Décimales avec une virgule : « 1,5 Go ».
- **Raccourcis clavier** :
  - Mac, à la façon d'Apple : « Commande + R », « Option + Commande + O », « Contrôle », « Maj ».
  - Windows, à la façon de Microsoft : « Ctrl+Maj+A », « Échap », « Inser », « Verr. maj ».
- **Variables intactes** : `{device}`, `{reader}`, `%@`, `%d`… ne se traduisent pas et ne disparaissent jamais.

## 4. Ce qu'on ne traduit pas

- Les noms de produits : AAE, AAE Remote, TalkBack, Backtalk, Jieshuo, VoiceOver, NVDA, JAWS, Wear OS, Android TV, Google TV, Googlebook, Google Play, Play Store, Sparkle, eSpeak NG.
- Les commandes et options de la ligne de commande (`aae start --cold`), les noms de paquets (`com.google.android.marvin.talkback`), les noms de fichiers, les adresses.
- « Mac » reste « Mac ». Le Narrateur de Windows garde son nom français, « Narrateur ».

## 5. Glossaire

Origine du terme : **TalkBack** (traduction officielle de TalkBack), **Android**, **Apple**, **MS** (Microsoft), **Émulateur** (documentation de l'émulateur Android), **proposé** (pas d'équivalent officiel, terme choisi pour AAE), ou **décidé** (choix tranché ensemble, voir section 6).

### Appareils et versions d'Android

| Anglais | Français | Origine |
|---|---|---|
| device | appareil | Android, Apple |
| virtual device | appareil virtuel | Émulateur |
| emulator | émulateur | Émulateur |
| Android version | version d'Android | Android |
| system image | image système | Émulateur |
| With Google services | Avec les services Google | proposé |
| With Google Play | Avec Google Play | proposé |
| Plain Android | Android sans services Google | proposé |
| preview (of an upcoming release) | préversion | proposé |
| 16 KB pages | pages de 16 Ko | Android |
| phone / tablet / foldable | téléphone / tablette / pliable | Android |
| watch | montre | Android |
| profile (small phone, phone…) | modèle | proposé |
| New Device… | Nouvel appareil… | proposé |
| Import Device… / Export… | Importer un appareil… / Exporter… | proposé |
| Copy… (clone a device) | Dupliquer… | proposé |
| Rename… | Renommer… | Apple |
| Delete… | Supprimer… | Apple |
| Emulator and Tools | Émulateur et outils | proposé |
| Update the Emulator and Tools | Mettre à jour l'émulateur et les outils | proposé |
| Installed Android Versions | Versions d'Android installées | proposé |

### Démarrage, arrêt et états

| Anglais | Français | Origine |
|---|---|---|
| Start / Stop / Restart | Démarrer / Arrêter / Redémarrer | Android |
| Create and Start | Créer et démarrer | proposé |
| quick boot | démarrage rapide | Émulateur |
| quick-boot snapshot | instantané de démarrage rapide | proposé |
| Cold Boot | Démarrage à froid | Émulateur |
| Wipe… | Réinitialiser… | décidé |
| snapshot | instantané | Émulateur |
| Save Snapshot… / Restore… | Enregistrer un instantané… / Restaurer… | proposé |
| running / stopped | en marche / arrêté | proposé |
| ready | prêt | proposé |
| first boot / first-boot setup | premier démarrage / configuration du premier démarrage | proposé |
| Speak Status | Énoncer l'état | Apple (« énoncer ») |

### Lecteur d'écran et gestes

| Anglais | Français | Origine |
|---|---|---|
| screen reader | lecteur d'écran | TalkBack |
| Choose Screen Reader… | Choisir un lecteur d'écran… | proposé |
| screen reader build | version de test du lecteur d'écran | décidé |
| Install Screen Reader Build… | Installer une version de test du lecteur d'écran… | proposé |
| Watch for New Builds… | Surveiller les nouvelles versions… | proposé |
| accessibility services | services d'accessibilité | Android |
| accessibility focus | focus | TalkBack |
| explore by touch | explorer au toucher | TalkBack |
| swipe right / left | balayer vers la droite / vers la gauche | TalkBack |
| double-tap | appuyer deux fois | TalkBack |
| three-finger tap | appuyer avec trois doigts | TalkBack |
| two-finger swipe | balayage à deux doigts | TalkBack |
| reading controls | contrôles de lecture | TalkBack |
| TalkBack menu | menu TalkBack | TalkBack |
| gesture mode / Use Gestures | mode gestes / Utiliser les gestes | proposé |
| touch point | point de contact | proposé |
| crown / Turn Crown Up | couronne / Tourner la couronne vers le haut | TalkBack |
| Fold / Unfold | Plier / Déplier | proposé |

### Clavier et mode appareil

| Anglais | Français | Origine |
|---|---|---|
| device mode | mode appareil | proposé |
| Return to the Mac | Revenir au Mac | proposé |
| Use Android Keyboard | Utiliser le clavier Android | proposé |
| keyboard layout | disposition du clavier | Apple, MS |
| Send Mac Clipboard to Device | Envoyer le presse-papiers du Mac à l'appareil | proposé |
| Type Mac Clipboard on Device | Taper le presse-papiers du Mac sur l'appareil | proposé |
| Copy Device Clipboard to Mac | Copier le presse-papiers de l'appareil sur le Mac | proposé |

### Parole et son

| Anglais | Français | Origine |
|---|---|---|
| text-to-speech | synthèse vocale | Android |
| speech engine | moteur de synthèse vocale | Android |
| speech rate | débit | Apple, NVDA |
| pitch | ton | Android |
| speech bridge | Voix de l'ordinateur (« pont vocal » seulement dans le README technique, si besoin) | décidé |
| speech log / Speech Log | journal de la parole | décidé |
| Record speech | Enregistrer la parole | proposé |
| Correct the pitch of older Android versions | Corriger le ton des anciennes versions d'Android | proposé |
| audio output | sortie audio | Apple |
| Device Volume Up / Down | Monter / Baisser le volume de l'appareil | proposé |
| Mute Device Audio | Couper le son de l'appareil | proposé |
| Check Audio / Check Microphone | Vérifier le son / Vérifier le micro | proposé |
| microphone | micro | Apple |

### Outils de test

| Anglais | Français | Origine |
|---|---|---|
| Accessibility Inspector | Inspecteur d'accessibilité | Apple |
| screen elements / element | éléments de l'écran / élément | proposé |
| Tree / Flat list | Arborescence / Liste à plat | proposé |
| Device Log | Journal de l'appareil | proposé |
| Errors / Warnings and errors / Info and above | Erreurs / Avertissements et erreurs / Informations et plus | proposé |
| Self-Test / Run Self-Test | Autodiagnostic / Lancer l'autodiagnostic | proposé |
| Save Diagnostic Report… | Enregistrer un rapport de diagnostic… | proposé |
| Save Screenshot… | Enregistrer une capture d'écran… | Apple |
| Install App… | Installer une app… | décidé |
| app | app (jamais « appli » ni « application ») | décidé |
| Force Stop | Forcer l'arrêt | Android |
| Clear Data… | Effacer les données… | Émulateur |
| Uninstall… | Désinstaller… | Android |
| Permissions… | Autorisations… | Android, Apple |
| Grant All / Leave All Off | Tout autoriser / Tout laisser désactivé | proposé |
| Special access | Accès spéciaux des apps | proposé |
| Open a Link / Open Link… | Ouvrir un lien / Ouvrir un lien… | proposé |
| Send an Intent / Send Intent… | Envoyer un intent / Envoyer un intent… | proposé (le mot « intent » est gardé, comme dans la doc Android) |
| broadcast | diffusion | proposé |
| shell | shell | proposé |

### Matériel

| Anglais | Français | Origine |
|---|---|---|
| Hardware… | Matériel… | proposé |
| memory / Memory, in megabytes | mémoire / Mémoire, en mégaoctets | proposé |
| processor cores | cœurs du processeur | proposé |
| storage | stockage | Android, Apple |
| screen size / density | taille de l'écran / densité | proposé |
| graphics adapter | carte graphique | proposé |
| software (graphics) | logiciel | proposé |
| virtualisation | virtualisation | MS |
| touchpad | pavé tactile | MS |

### Batterie, position, téléphone et réseau

| Anglais | Français | Origine |
|---|---|---|
| Battery / Battery level / Charging | Batterie / Niveau de la batterie / En charge | Android |
| Location / Set Location | Position / Définir la position | Android |
| route (GPX) / Route speed | itinéraire / Vitesse de l'itinéraire | proposé |
| Phone Call / Call the Device | Appel / Appeler l'appareil | proposé |
| Answer the Device's Call / Hang Up | Répondre à l'appel de l'appareil / Raccrocher | proposé |
| Text Message / Send Text Message | SMS / Envoyer un SMS | proposé |
| Network / Wi-Fi / Mobile data / Airplane mode | Réseau / Wi-Fi / Données mobiles / Mode Avion | Android |
| Touch Fingerprint Sensor | Toucher le capteur d'empreinte | proposé |
| Shake the Device | Secouer l'appareil | proposé |
| Display and Language / Set Language | Affichage et langue / Définir la langue | proposé |

### Android : boutons et écrans

| Anglais | Français | Origine |
|---|---|---|
| Back / Home | Retour / Accueil | TalkBack, Android |
| Recent Apps | Apps récentes (« Récents » pour désigner le bouton d'Android lui-même) | décidé |
| Notifications | Notifications | TalkBack |
| Quick Settings | Réglages rapides | TalkBack |
| Power Button | Bouton Marche/Arrêt | proposé |
| status bar / navigation bar | barre d'état / barre de navigation | Android |
| Settings (Android) | Paramètres | Android |
| developer options / USB debugging | options pour les développeurs / débogage USB | Android |

### AAE Remote

| Anglais | Français | Origine |
|---|---|---|
| Serve Devices to Phones… | Partager les appareils avec des téléphones… | proposé |
| Serve at login, without AAE open | Partager dès l'ouverture de session, même AAE fermé | proposé |
| pairing code / New Pairing Code | code d'association / Nouveau code d'association | proposé |
| Paired Phones / Unpair | Téléphones associés / Dissocier | proposé |

### Commandes génériques

| Anglais | Français | Origine |
|---|---|---|
| OK / Cancel | OK / Annuler | MS, TalkBack |
| Done | OK | décidé |
| Save / Open / Close | Enregistrer / Ouvrir / Fermer | Apple |
| Copy / Copy All / Copy Line | Copier / Tout copier / Copier la ligne | proposé |
| Clear | Effacer | proposé |
| Refresh | Actualiser | proposé |
| Try Again / Run Again | Réessayer / Relancer | proposé |
| Pause / Resume | Pause / Reprendre | proposé |
| Accept / Decline | Accepter / Refuser | proposé |
| Continue Without | Continuer sans | proposé |
| Check for Updates… | Rechercher des mises à jour… | Apple |
| Learn more | En savoir plus | TalkBack |
| None / None yet. | Aucun / Aucun pour l'instant. | proposé |
| System default | Par défaut du système | proposé |

### macOS et Windows

| Anglais | Français | Origine |
|---|---|---|
| System Settings / Privacy & Security | Réglages Système / Confidentialité et sécurité | Apple |
| Accessibility / Input Monitoring / Automation | Accessibilité / Surveillance de l'entrée / Automatisation | Apple |
| Keychain | trousseau | Apple |
| VoiceOver modifier (VO) | touche de modification VoiceOver (VO) | Apple |
| VoiceOver Utility | Utilitaire VoiceOver | Apple |
| Spoken Content | Lire et énoncer | Apple (macOS 26 et suivants) |
| system voice | voix système | Apple |
| Windows Settings | Paramètres | MS |
| Turn Windows features on or off | Activer ou désactiver des fonctionnalités Windows | MS (« des » ou « les » : à vérifier sur un Windows en français) |
| Windows Hypervisor Platform | Plateforme de l'hyperviseur Windows | à vérifier sur un Windows en français |
| NVDA key | touche de modification NVDA | NVDA |
| Narrator | Narrateur | MS |

## 6. Choix tranchés

| Question | Décision |
|---|---|
| app, appli ou application | **app**, partout. Donc « Apps récentes » et « Accès spéciaux des apps ». |
| Speech bridge | **Voix de l'ordinateur** dans les apps ; « pont vocal » seulement dans le README technique, si besoin. |
| Speech log | **journal de la parole**, cohérent avec « journal de l'appareil ». |
| Screen reader build | **version de test** du lecteur d'écran. |
| Wipe | **Réinitialiser…** |
| Recent Apps | **Apps récentes** dans les menus ; « Récents » pour le bouton d'Android lui-même. |
| Done | **OK** |
| Raccourcis Mac | **Commande + R**, comme Apple. |

## 7. Termes fixés pendant la traduction

| Anglais | Français |
|---|---|
| AAE's helper (l'app qu'AAE installe sur l'appareil) | l'app auxiliaire d'AAE |
| Graphics (réglage du matériel) | Graphismes |
| Watch (type d'appareil) / Watch (bouton qui lance la surveillance) | Montre / Surveiller (contexte « start watching for builds ») |
| Copy (bouton qui duplique un appareil) / Copy (copier du texte) | Dupliquer (contexte « duplicate a device ») / Copier |
| Device Volume Up / Down (touches de volume d'Android) | Monter / Baisser le volume de l'appareil |
| Turn Device Audio Up / Down (son de l'appareil sur l'ordinateur) | Monter / Baisser le son de l'appareil sur l'ordinateur |
| Watch Another… | Surveiller une autre app… |
| diagnostic report (les lignes du rapport) | restent en anglais : le rapport est destiné au mainteneur |

Un fragment inséré dans une autre phrase (« Impossible de {what} ») se traduit pour s'y emboîter : pas de « de les », pas de « de ouvrir ». Quand c'est impossible, on reformule la phrase d'accueil (« Téléchargement en cours : {what} »).
