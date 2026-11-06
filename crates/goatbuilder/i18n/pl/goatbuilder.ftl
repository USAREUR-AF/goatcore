### goatbuilder — pl (pl)
### source: ops/scripts/translate-ftl.py (machine)
### review: machine — see crates/goat-cli/i18n-provenance.toml

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-about = Zbuduj i wdróż środowisko goat: zadeklaruj, doprowadź do zbieżności, zweryfikuj — w jednym kontrakcie wdrożeniowym
help-flag-output = Format wyjścia: tabela dla człowieka; JSON lub YAML dla programu
help-flag-repo = Katalog główny repozytorium. Domyślnie checkout, w którym uruchomiono ten program
help-flag-yes = Potwierdza czasownik deklaratywny bez pytania (dla automatyzacji)
help-flag-frozen = Odrzuca każdy przebieg, który wymagałby fast-forwardu checkoutu (ścisłe C3)
help-flag-mock = Uruchom na katalogu fixture zamiast na hostach: bez rejestru, bez transportu, deterministycznie (goatbuilder-conformance.md §"The fixture")
help-flag-lang = Język pomocy i wyjaśnień błędów (tag, np. fr, uk, de); wyjście czytane przez pipeline zostaje po angielsku. Domyślnie: ustawienia regionalne systemu
help-cmd-validate = Sprawdź konfigurację względem wersjonowanego schematu i reguł międzypolowych, offline
help-cmd-doctor = Sprawdź, czego wymaga przebieg, zanim cokolwiek zostanie wycięte: zależności, aktualność checkoutu, transport każdego środowiska oraz to, że playbook każdego komponentu wskazuje jednoznaczny tryb docelowy
help-cmd-status = Zadeklarowane vs zapisane, per maszyna: converged, pending lub unknown (C7, C8). Tylko odczyt. Kod wyjścia 3, dopóki jakaś maszyna nie jest converged
help-cmd-deploy = Wdróż na jedno środowisko pin zadeklarowany w main i to udowodnij: z pinów rozwiąż komponent i grupy środowiska, play i jego tryb docelowy, każdy sekret, który play szablonuje, oraz transport; uruchom play przez ansible-preflight; odczytaj ledger wdrożeń z każdego hosta i uznaj za gotowe dopiero, gdy najnowszy wiersz wskazuje zadeklarowany digest z action=deploy (C7). Na fixture (`--mock`) hosty zastępuje skryptowy converger
help-cmd-release = Przeprowadź release candidate od tagu na main przez żywą walidację na goatnecie dev do szczebla preprod (ADR 1173); po soaku, z `--from prod`, awansuj go, włącz do fleet:, wdróż prod i ponownie zweryfikuj każdy szczebel. Zastępuje ops/scripts/rc.sh krok po kroku
help-cmd-rollback = Cofnij wdrożony komponent do jego N-tego najnowszego refa, według typu artefaktu, z ledgerem na hoście jako źródłem prawdy (ADR 1060). Zastępuje ops/scripts/rollback.sh: kod 3 odmowa jako niebezpieczne (data, cert, forward-only, reversible:false), 4 tryb z przewodnikiem (config, acl)
help-cmd-fastlane = Jedno polecenie od zmergowanego commita do każdego goatnetu (ADR 1127): świadomy wyjątek od drabiny. Zastępuje ops/scripts/fastlane.sh krok po kroku; odmawia przy niepinowanym digeście lub otwartym zestawie awansu, a wdrożenia uruchamia przez ten program
help-cmd-standup = Lokalizacja od zera do zweryfikowanej: fazy standupu w kolejności z doktryny, z bramką fail-closed na obu końcach, host obserwowalności jako równoległa gałąź, klasy rescue i powierzchnie między goatnetami na końcu (ADR 1183). Zastępuje ops/scripts/goat-standup.sh krok po kroku. Na fixture (`--mock`) każdy komponent każdej klasy jest doprowadzany do zbieżności przez skryptowy converger, a czas od zadeklarowania do zweryfikowania to miara goatnetu jednogodzinnego
help-cmd-join-node = Kolejna maszyna danej klasy na żywej lokalizacji, od zadeklarowania przez converged do zweryfikowania: wykonana sekwencja z docs/tutorials/02-first-federated-node.md jako ponumerowane fazy z bramkami (osiągalność, klucz, najpierw prace w repo, baseline, kernelowy WireGuard przed i po enrolmencie, kernelowa płaszczyzna ACL jako bramka enrolmentu, converge klasy, akceptacja)
help-cmd-federate = Suwerenny peering zadeklarowany, doprowadzony do zbieżności i zweryfikowany (ADR 1157): zmierzona nasza połowa, pakiety wymienione poza pasmem, jedna umowa złożona i zweryfikowana, z niej wybite liście, reguły wyrenderowane przez jedynego pisarza, wdrożona krawędź, otwarty transport wybrany przez umowę, akceptacja we wszystkich czterech poziomach. Zastępuje ops/scripts/peering-standup.sh krok po kroku
help-cmd-pins = JEDEN przebieg rozwiązywania, DWA renderingi: pins.yml w group_vars każdego wymienionego członka i artefakt IntentV1 każdego wymienionego goatnetu, z ops/pins/fleet-pins.yaml. Zastępuje ops/pins/generate-pins.py co do bajta. `--check` waliduje, regeneruje i odrzuca dryf, niepodpisany lub naruszony intent oraz pin objęty zarządzaniem, zdefiniowany poza pins.yml (kod 1)
help-cmd-promote = Prowadź i obserwuj awans wydań na drabinie goatnetów (ADR 1120, 1125): status, otwarcie kandydata, awans, awans zestawu, ścieżka hotfixu, review, gotowość, porzucenie, check. Zastępuje ops/release/graduate.py krok po kroku. Nic tu nie awansuje samo: mutacja rusza tylko na wywołanie, wypisuje, co zrobiła, i bez terminala wymaga --yes (C10, ADR 1177)
help-cmd-explain = Zinterpretuj błąd: kod (`GB-007`, `X-SSH-001`, `E_OFF_MESH`) lub dowolny tekst błędu — własny tego programu albo cudze słowa, które przeniósł (ansible, ssh, op, aws, git, gh, cosign, docker, netbird). Wypisuje, co się stało, dlaczego, co robić i gdzie czytać dalej; `-` czyta tekst ze stdin

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-teardown = Lokacja od zweryfikowanej do zera — lustro standup, zapisy zostają (burn-down dopisuje wiersze teardown, nigdy nie kasuje). Na fixture (`--mock`) próbuje burn-down w odwrotnej kolejności faz; na prawdziwych hostach wypisuje kolejność i odmawia wykonania, dopóki każda faza nie zadeklaruje swojego teardown play (docs/design/goatbuilder-speed.md)
help-cmd-tutor = Lekcje w stylu vimtutor: każda podaje dokładne polecenie do uruchomienia na jednorazowej kopii fixture (lub, tylko do odczytu, na tym checkoucie), po czym sprawdza, co to polecenie faktycznie zrobiło — kod wyjścia i wiersze — zanim odblokuje następną. `tutor` pokazuje bieżącą lekcję i oznacza ją jako ukończoną, gdy Twoje uruchomienia ją spełnią; `tutor <n>`, `--list`, `--reset`
help-cmd-make = Ukryte (src/fun.rs)
help-cmd-argan = Ukryte (src/fun.rs)
help-cmd-bound = Ukryte (crates/goat-fun)
help-cmd-secrets = Zadeklarowany stan sekretów (ADR 1204): który magazyn obowiązuje (1password, openbao, sops, git-crypt, env-file, env), co jest dostępne, jak dostarczyć brakujące i własne kontrole magazynu. `init --env <goatnet>` zapisuje plik env dla quickstartu zero-SaaS: silna losowa wartość dla każdej zmiennej, którą szablonują pleje goatnetu, 0600, w gitignore, bez rotowania wartości już obecnej. Ten sam moduł co ops/scripts/with-secrets

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-verify = Odczytuje najnowszy wiersz ledgera wdrożeń każdego hosta dla jednego komponentu i sprawdza, czy wskazuje ref zadeklarowany w main (C7). Tylko odczyt; kod wyjścia 3, dopóki jakikolwiek host jest w stanie pending lub unknown. To samo sprawdzenie uruchamia `deploy --verify-only`
help-cmd-diff = Czym różni się teraz od main (C8): zadeklarowany w main ref względem najnowszego wiersza ledgera każdego hosta, dla jednego komponentu lub, bez podania komponentu, dla każdego komponentu środowiska, odczyt jednym przebiegiem na grupę członków. Tylko odczyt; kod wyjścia 3 przy dowolnym dryfie lub nieczytelnym hoście
help-cmd-history = Ledger wdrożeń jako oś czasu, od najnowszych, na każdym hoście środowiska (C8): każdy wiersz to osobny zapis z maszyną, czasem, akcją (deploy lub rollback) i wersją. Poprzednia wersja zapisu i operator, który go wykonał, są w `-o json`. Tylko odczyt
help-cmd-plan = Zapisuje do pliku planu, do przeglądu, co zrobiłby czasownik deklaratywny: zadeklarowane refy, play i cel, ledger, commit main, transport (deploy); typ artefaktu i docelowy ref (rollback). Nie zmienia stanu żadnego hosta. Uruchom po przeglądzie przez `apply`
help-cmd-apply = Wykonuje dokładnie zrecenzowany plan. Odmawia (kod wyjścia 1, ze wskazaniem, co się zmieniło), gdy deklaracja main, commit pins, play lub cel rollbacku zmieniły się od zapisania planu; zmieniony transport jest zgłaszany, a nie odrzucany
help-cmd-adopt = Deklaruje to, co środowisko już uruchamia: odczytuje ledgery każdego hosta i proponuje odchylenie pins dla każdego komponentu, którego działający ref różni się od deklaracji w main. Domyślnie raport; z --yes zmiany trafiają do świeżego worktree main do przeglądu. Nigdy nie wdraża

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
tutor-not-yet = Jeszcze nie: { $why }
tutor-lesson-done = Lekcja { $n } ukończona: { $title }.
tutor-all-done-reset = Wszystkie lekcje ukończone. `goatbuilder tutor --reset` zaczyna od nowa; tutoriale idą dalej: docs/tutorials/.
tutor-all-done = Wszystkie lekcje ukończone. Tutoriale idą dalej: docs/tutorials/.
tutor-workspace = Przestrzeń robocza: { $path }


# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-assay = Sprawdź, czy goatnet został zbudowany poprawnie, warstwa po warstwie: każdy test odbiorczy właściwy dla danego site'u, w tym testy, które operator uruchamia przez CLI goat. Faza odbioru ze standupu jako osobne polecenie. Tylko odczyt; kod wyjścia 3, dopóki jakikolwiek test kończy się błędem lub nie może dać odpowiedzi
# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-flag-offline = Wyizolowany ośrodek: nie pobieraj gałęzi głównej; oceń kopię roboczą względem tej z pamięci podręcznej i zapisz przebieg jako offline (C3; ops/scripts/fresh-source.sh)

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-endstate = Sprawdza, czy goatnet jest w zadeklarowanym stanie docelowym: każda usługa, którą deklaruje, jest wdrożona, działa, jest zdrowa i wykonuje swoje zadanie, jej krawędzie zero-trust są podłączone, a nic niezadeklarowanego nie działa. Tylko odczyt; kod wyjścia 3, dopóki coś jest nie tak
help-cmd-survey = Spójność oprogramowania wewnątrz każdego goatnetu i między goatnetami drabiny, odczytana w jednym równoległym przebiegu. Tylko odczyt

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-survey-long =
    Spójność oprogramowania w obrębie każdego goatnetu i między goatnetami w drabinie, odczytana w jednym przebiegu równoległym. Tylko odczyt.

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-cmd-update = Zastępuje ten plik binarny wydaniem serwowanym przez jego goatnet pod `get.<site>.<zone>/goatbuilder/`: pobrany plik jest sprawdzany względem `.sha256` i podpisanego `SHA256SUMS`, zanim cokolwiek zostanie ruszone, a podmiana jest atomowa. Już aktualny: jeden wiersz, kod wyjścia 0. `--check` tylko odczytuje (kod 3, gdy wersja jest starsza). Adres: `GOATBUILDER_UPDATE_URL`, w razie braku `--site` z jego group_vars, a w razie braku — ten z ostatniej aktualizacji
tutor-prereq = Zanim zaczniesz: { $prereq }

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-flag-color = Koloruj glif ok/FAIL i werdykt w tabeli (auto|always|never). `auto` koloruje tylko w terminalu; NO_COLOR i CLICOLOR_FORCE mają pierwszeństwo. JSON i YAML nigdy nie są kolorowane
help-flag-verbose = Loguj na stderr, co robi uruchomienie — zewnętrzne polecenia, które wywołuje, i fazę sieciową `update` — dzięki temu `-o json|yaml` zost
help-flag-timeout = Sekundy ograniczające fazę łączenia/osiągalności przed szybkim niepowodzeniem — jedyną fazę, która może się zablokować na nieosiągalnym hoście (pobranie serwowanej wersji przez `update`). NIE ogranicza własnego przebiegu playbooka ani pobierania binarki (ograniczonego przez zastój). Czyta też GOATBUILDER_TIMEOUT
help-cmd-completion = Wypisz skrypt uzupełniania powłoki na stdout (`goatbuilder completion zsh >> ~/.zshrc`). Dla dynamicznych kandydatów, których statyczny skrypt nie zna — witryn, komponentów i goatnetów zadeklarowanych w tym checkoucie — podłącz hook do `goatbuilder _complete` (zobacz nagłówek wygenerowanego skryptu)
help-cmd-docs = Dokumentacja poleceń: `goatbuilder docs` na nią wskazuje; `--markdown` emituje całość, przejętą z żywego drzewa poleceń, więc nigdy nie może się rozejść z `--help`
help-cmd-_complete = Ukryte: emituje dynamicznych kandydatów uzupełniania dla hooków powłoki, po jednym na linię (witryny, komponenty i goatnety zadeklarowane w tym checkoucie). Tylko odczyt, nigdy nie zawodzi głośno — hook uzupełniania nie może uszkodzić promptu

# machine-translated additions (2026-10-31) — review per i18n-provenance.toml
help-cmd-preflight = Przed wdrożeniem: odczyt gotowości i odwracalności (ADR 1211): trzy fakty — State (stateless | stateful), Rollback (pin-revert | restore-required | none) i Scope (single | coupled) — deklarowane per komponent i FAIL-CLOSED, gdy nie zadeklarowano, wraz z częścią o gotowości, którą potwierdzają inne polecenia (ref do pobrania, pins --check, bramka sekretów, świeże źródło, zadeklarowany stan docelowy) oraz walidacją faktu rollback (rollback --dry-run dla pin-revert, okno retencji dla restore-required). `deploy` uruchamia go jako pierwszy; ocena uczciwości, nigdy fałszywy zielony. Tylko odczyt; kod wyjścia 3, gdy nie jest czysto

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-l1-title = Sprawdź swoją konfigurację
tutor-l1-s1 = Sprawdź plik konfiguracyjny
tutor-l2-title = Odczytaj stan: converged, pending, unknown
tutor-l2-text =
    Polecenie goatbuildera `status` porównuje dwie rzeczy dla każdej maszyny: wersję, którą deklaruje dla niej main, oraz wersję, którą jej własny zapis mówi, że uruchamia. Nic nie zmienia.

    Każda maszyna jest w jednym z trzech stanów:
    - converged: jej zapis podaje zadeklarowaną wersję. To jest zrobione.
    - pending: jej zapis podaje starszą wersję, albo jeszcze nic.
    - unknown: goatbuilder nie może odczytać jej zapisu.

    Tylko converged liczy się jako zrobione. Jeśli jakaś maszyna jest pending albo unknown, `status` kończy się kodem wyjścia 3.

    Od tej lekcji `--mock` wskazuje goatbuilderowi kopię ćwiczebną w twoim folderze tutora, zamiast prawdziwych maszyn. Nic, co tam robisz, nie dociera do prawdziwej maszyny.

    Na `dev` w kopii ćwiczebnej jeszcze nic nie wdrożono. Oczekuj więc czterech maszyn pending i jednej unknown. Maszyny converged zobaczysz w lekcji 3.
tutor-l2-s1 = Odczytaj stan dev
tutor-l3-title = Wdrożenie: zaplanuj, wykonaj, udowodnij
tutor-l3-text =
    Wdrażanie umieszcza zadeklarowaną wersję na każdej maszynie, a potem czyta zapis każdej maszyny, aby to udowodnić. Wdrożenie jest zrobione tylko wtedy, gdy zapis każdej maszyny podaje tę wersję. Wdrożenie, które przebiegło bez błędów, to nie wystarczy.

    Wdrożysz `portal` na `preprod` kopii ćwiczebnej. `--dry-run` pokazuje, co by się stało, i nic nie zmienia. `--ack` zapisuje, kto to zrobił; polecenie wpisuje twoje imię i nazwisko z git. Po wdrożeniu `status` powinien pokazać obie maszyny jako `ok` i converged.
tutor-l3-s1 = Zaplanuj wdrożenie
tutor-l3-s2 = Wdróż portal na preprod
tutor-l3-s3 = Sprawdź, że obie maszyny są converged
tutor-l4-title = Dryf i historia
tutor-l4-text =
    Dwa polecenia pomagają zrozumieć goatnet przed zmianą i po niej. Oba tylko czytają.
    - Polecenie goatbuildera `diff` wypisuje każdą maszynę, której zapis różni się od zadeklarowanej wersji. Taką różnicę nazywamy dryfem.
    - Polecenie goatbuildera `history` wypisuje każdą wersję, jaką komponent miał na każdej maszynie, od najnowszej.

    Na `dev` każda maszyna jest w tyle. Historia `portal` na `preprod` teraz zawiera twoje wdrożenie z lekcji 3.
tutor-l4-s1 = Wypisz dryf na dev
tutor-l4-s2 = Przeczytaj historię portal na preprod
tutor-l5-title = Cofnij wersję i zobacz, jak niebezpieczne cofnięcie zostaje odrzucone
tutor-l5-text =
    Cofnięcie deklaruje wcześniejszą wersję i wdraża ją jak każdą inną wersję. CLI goatbuildera potem czyta zapis każdej maszyny, aby to udowodnić.

    Niektórych zmian nie da się tak odwrócić. Wdrożenie starszej wersji nie może odwrócić danych, które zapisał komponent, certyfikatu, ani zmiany, która idzie tylko do przodu. CLI goatbuildera odmawia takich cofnięć z kodem wyjścia 3 i mówi, co zrobić zamiast tego.

    Komponent `db-schema` kopii ćwiczebnej przechowuje dane, więc goatbuilder odmawia jego cofnięcia. Ta odmowa jest oczekiwanym wynikiem.
tutor-l5-s1 = Zaplanuj cofnięcie
tutor-l5-s2 = Cofnij portal o jedną wersję
tutor-l5-s3 = Spróbuj cofnąć db-schema, czego goatbuilder odmawia
tutor-l6-title = Wydanie nowej wersji: dev, potem preprod, potem prod
tutor-l6-s1 = Zobacz, gdzie jest każdy komponent
tutor-l6-s2 = Uruchom suchy przebieg wydania z dev
tutor-l6-s3 = Uruchom suchy przebieg kroku prod
tutor-l7-title = Fastlane, i kiedy go nie używać
tutor-l7-text =
    Fastlane umieszcza jedną wersję na wszystkich goatnetach naraz. Pomija czas czekania na preprod i zapisy wydania.

    Używaj go tylko dla pilnej poprawki, którą możesz odwrócić, wracając do poprzedniej wersji. Nigdy nie używaj go do zmiany bazy danych, zmiany tożsamości albo dostępu, ani do niczego, czego nie da się tak odwrócić.

    Fastlane odmawia wersji, której CI nie zbudowało, oraz komponentu, który jest częścią wydania wciąż w toku.

    Ogólna forma to `goatbuilder fastlane <component> --ref <version> --reason <why>`. Uruchom przykład poniżej. Ma `--dry-run`, więc nic się nie zmienia. Potrzebuje `gh`.
tutor-l7-s1 = Uruchom suchy przebieg fastlane
tutor-l8-title = Pins: znajdź dryf i napraw go właściwym sposobem
tutor-l8-text =
    Jeden plik ustala, jaką wersję uruchamia każdy goatnet: `ops/pins/fleet-pins.yaml`. CLI goatbuildera generuje z niego pozostałe pliki wersji: pliki `pins.yml` oraz podpisane pliki intent.

    Polecenie goatbuildera `pins --check` porównuje wygenerowane pliki z `fleet-pins.yaml` i odmawia dla każdego pliku, który się różni. Polecenie goatbuildera `pins` generuje je ponownie.

    Twój folder tutora ma kopię repozytorium, w której ktoś ręcznie zmienił jeden plik `pins.yml`. Nigdy nie poprawiaj ręcznej zmiany kolejną ręczną zmianą. Zmień `fleet-pins.yaml`, a potem wygeneruj ponownie.
tutor-l8-s1 = Sprawdź ten checkout
tutor-l8-s2 = Sprawdź kopię i znajdź ręczną zmianę
tutor-l8-s3 = Wygeneruj ponownie pliki kopii
tutor-l8-s4 = Sprawdź kopię znowu
tutor-l9-title = Wyjaśnij błąd
tutor-l9-s2 = Wyjaśnij błąd za pomocą jego kodu
tutor-l9-s3 = Wyjaśnij go za pomocą jego tekstu
tutor-l10-title = Tutorial 1: zbuduj pierwszą lokalizację nowego goatnetu
tutor-l10-text =
    Ta lekcja buduje cały goatnet w kopii ćwiczebnej. Przykładowy goatnet to `zagros`. Jego pierwsza lokalizacja, `alpha`, ma dwie maszyny core, jedną maszynę monitoringu i jedną maszynę ratunkową.

    Polecenie goatbuildera `standup` uruchamia każdą maszynę w ustalonym porządku i sprawdza każdy zapis. Jego ostatnia linia mówi, że lokalizacja jest gotowa. Na prawdziwych maszynach ta linia pokazuje też, ile to trwało.

    Pełny tutorial to `docs/tutorials/01-new-goatnet.md`.
tutor-l10-s1 = Zbuduj pierwszą lokalizację, alpha
tutor-l10-s2 = Sprawdź wynik
tutor-l11-title = Tutorial 2: dodaj drugą lokalizację
tutor-l11-text =
    Ta lekcja zaczyna się tam, gdzie skończył się tutorial 1. Dodaje drugą lokalizację, `beta`, w nowej kopii ćwiczebnej, `zagros-2`.

    Druga lokalizacja kopiuje konta użytkowników z pierwszej lokalizacji. Jej standup zmienia też raz pierwszą lokalizację, aby to kopiowanie działało w obie strony. Robi to krok o nazwie `alpha/core`.

    Pełny tutorial to `docs/tutorials/02-first-federated-node.md`.
tutor-l11-s1 = Zbuduj drugą lokalizację, beta
tutor-l11-s2 = Sprawdź obie lokalizacje
tutor-l12-s1 = Zaplanuj dodanie jednej maszyny
tutor-l13-s1 = Zaplanuj federację
tutor-l14-title = Aktualizuj goatbuilder
tutor-l14-s1 = Zapytaj, czy istnieje nowszy goatbuilder
tutor-l15-title = Assay: sprawdź, jak został zbudowany goatnet
tutor-l15-text =
    Polecenie goatbuildera `assay` sprawdza, czy goatnet został zbudowany poprawnie, warstwa po warstwie, za pomocą testów akceptacyjnych. Każdy test dostaje jeden z trzech wyników:
    - PASS: test przeszedł.
    - FAIL: goatnet ma problem.
    - UNKNOWN: nic nie mogło odpowiedzieć na test.

    Tylko PASS się liczy. Każdy FAIL albo UNKNOWN sprawia, że kod wyjścia to 3.

    `--plan` wypisuje testy, które dotyczą goatnetu, warstwa po warstwie, i nie dotyka żadnej maszyny. Użyj go, aby zobaczyć, co obejmowałby prawdziwy assay. Ogólna forma to `goatbuilder assay <goatnet> --plan`.
tutor-l16-title = Survey: czy wersje są zgodne?
tutor-l16-text =
    Polecenie goatbuildera `survey` sprawdza w jednym przebiegu, czy te same wersje działają tam, gdzie powinny:
    - W każdym goatnecie: czy każda maszyna uruchamia zadeklarowaną wersję, i czy maszyny, które powinny się zgadzać, naprawdę się zgadzają?
    - Między dev, preprod i prod: jeden wiersz na komponent, jedna kolumna na goatnet.

    Różnica, której oczekuje proces wydania, jest zgłaszana, ale nie jest błędem. Przykładem jest nowa wersja, która jest na dev, ale jeszcze nie na prod. Różnica, której nic nie deklaruje, odczytywana jest jako `difference not declared`. Maszyna, której goatbuilder nie może odczytać, jest unknown, nigdy consistent.

    Uruchom go na kopii ćwiczebnej. Jej `dev` jest w tyle celowo, więc oczekuj wyników i kodu wyjścia 3.
tutor-l16-s1 = Uruchom survey na kopii ćwiczebnej
tutor-l17-title = Endstate: czy goatnet uruchamia dokładnie to, co deklaruje?
tutor-l17-text =
    Polecenie goatbuildera `endstate` sprawdza dwie rzeczy w goatnecie:
    1. Każda usługa, którą deklaruje, jest wdrożona, działa, jest zdrowa i poprawnie połączona.
    2. Nic nie działa, czego nie deklaruje. Niezadeklarowaną usługę nazywamy sierotą.

    CLI goatbuildera ustala, co lokalizacja powinna uruchamiać, na podstawie własnych ustawień lokalizacji, nie z osobnej listy.

    Sprawdzenie, które nie ma nic do odczytania, jest oznaczone UNOBSERVED. To jest szczera odpowiedź, nie błąd. FAIL albo sierota sprawiają, że kod wyjścia to 3.

    Ogólna forma to `goatbuilder endstate <goatnet>`. Polecenie poniżej sprawdza przykładową lokalizację z zapisanych danych, więc nie dotyka żadnej maszyny.
tutor-l17-s1 = Sprawdź przykładową lokalizację
tutor-l18-title = Bonus: bądź ciekawy
tutor-l18-text =
    Ta lekcja jest opcjonalna. Uruchom polecenie poniżej, a tutor się skończy.

    CLI goatbuildera ma kilka małych ukrytych funkcji. Nie są w pomocy ani w tych lekcjach. Niektóre reagują na słowa, które długoletni użytkownicy Uniksa często wpisują w nowy program, żeby zobaczyć, co się stanie. Większość rzeczy, które wypróbujesz, nic nie zrobi.

    Ta lekcja nie mówi, czym one są. Znalezienie ich to cała zabawa.

    Polecenie poniżej wypisuje każdy błąd, który goatbuilder umie wyjaśnić. To dobre miejsce, aby zacząć odkrywanie.
tutor-l18-s1 = Wypisz każdy błąd, który goatbuilder umie wyjaśnić
tutor-heading = Lekcja { $n } z { $total } · { $title }
tutor-setup = Najpierw uruchom to raz w tym terminalu:
tutor-not-recording = Ten terminal nie zapisuje twoich przebiegów, więc tutor nie może zaznaczać twoich kroków. Uruchom to raz, a potem uruchom kroki:
tutor-type = Wykonaj te kroki w swoim terminalu ({ $met } z { $total } zrobione):
tutor-rerun = Po każdym kroku uruchom `goatbuilder tutor` znowu. Sprawdza, co zrobiłeś, i zaznacza krok.

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-welcome = Witaj w GOATBUILDER Tutor
tutor-bar-progress = Postęp
tutor-bar-done = { $done } z { $total } lekcji zrobione
tutor-after-step = Krok { $met } z { $total } zrobiony. Następny:
tutor-after-lesson = Lekcja { $n } jest gotowa. Uruchom `goatbuilder tutor`, aby zacząć lekcję { $next }. Aby przeczytać ją ponownie: `goatbuilder tutor --lesson { $n }`.
tutor-after-final = Lekcja { $n } jest gotowa, a tutor także. Uruchom `goatbuilder tutor`, aby zobaczyć, że jest oznaczony jako zrobiony. Aby przeczytać ją ponownie: `goatbuilder tutor --lesson { $n }`.

tutor-review = Ta lekcja jest zakończona, więc ta strona służy tylko do czytania. Uruchom `goatbuilder tutor`, aby wrócić tam, gdzie jesteś.
tutor-other-lessons = Aby przeczytać lekcję ponownie: `goatbuilder tutor --lesson 1`. Aby zobaczyć je wszystkie: `goatbuilder tutor --list`.
tutor-l12-title = Dodaj maszynę do działającej lokalizacji
tutor-l12-text =
    Polecenie goatbuildera `join-node` dodaje jeszcze jedną maszynę do lokalizacji, która już działa. Najpierw wypisuje plan, z jednym wierszem na fazę. Każda faza ma typ:
    - tool: goatbuilder uruchamia skrypt.
    - ansible: goatbuilder uruchamia playbook.
    - gate: kontrola, która musi przejść przed następną fazą.
    - manual: robisz to sam, a przebieg się zatrzymuje.

    Polecenie poniżej ma `--dry-run`. Czyta ten checkout i nic nie zmienia. Pod tabelą przeczytaj, co robisz ręcznie.
tutor-l13-title = Połącz się z goatnetem innej organizacji
tutor-l13-text =
    Polecenie goatbuildera `federate` łączy twój goatnet z goatnetem, który uruchamia inna organizacja. Każda strona przygotowuje swoją połowę umowy. Wymieniacie się połowami, a każda strona łączy je w tę samą umowę.

    Potwierdź klucz drugiej strony w inny sposób, na przykład rozmową, której ufasz. Klucz, który przychodzi w ich pakiecie, niczego nie dowodzi.

    Polecenie poniżej ma `--dry-run`, więc nic nie zmienia. Prawdziwy przebieg zatrzymuje się na fazie 4, bo tę fazę wykonujesz ręcznie.
# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-where = To polecenie nie jest częścią lekcji { $n }. Uruchom `goatbuilder tutor`, aby zobaczyć, gdzie jesteś.
tutor-l1-s2 = Zapytaj doktora o jeden goatnet
tutor-l1-text =
    CLI goatbuilder buduje, wdraża i wydaje oprogramowanie GOAT. Zanim go użyjesz, sprawdź swoją konfigurację dwoma poleceniami:
    - Polecenie goatbuildera `validate` sprawdza plik konfiguracyjny `ops/goatbuilder/goatbuilder.yaml`. Czyta tylko ten checkout i nie potrzebuje sieci.
    - Polecenie goatbuildera `doctor` sprawdza, czego potrzebuje prawdziwe wdrożenie: twoje narzędzia, twoje logowania, czy ten checkout zgadza się z main i jak goatbuilder dociera do każdego goatnetu. Goatnet to jedno środowisko GOAT.

    Polecenie goatbuildera `doctor` wypisuje, ile sprawdzeń przeszło, a potem każdy problem z jego poprawką. W tej lekcji problem jest informacją, nie błędem. Przeczytaj poprawki, ale nie musisz ich teraz stosować.

    Większość lekcji nie wymaga logowania. Lekcje od 2 do 5, 10, 11 i 16 używają kopii ćwiczebnej. Lekcje 8, 9, 12, 13, 15, 17 i 18 tylko czytają ten checkout. Lekcja 6 wymaga `gh` i mesha, lekcja 7 wymaga `gh`, a lekcja 14 wymaga mesha. Żadna lekcja nie wymaga 1Password.
tutor-l14-text =
    CLI goatbuilder może zaktualizować się z kopii, którą twój goatnet udostępnia pod `get.<site>.<zone>/goatbuilder/`.

    Polecenie goatbuildera `update --check` tylko czyta. Porównuje twoją wersję z udostępnianą wersją i nic nie zmienia. Kod wyjścia 0 oznacza, że masz aktualną wersję. Kod wyjścia 3 oznacza, że istnieje nowsza wersja.

    Prawdziwa aktualizacja, bez `--check`, weryfikuje sumę kontrolną i podpis pobranego pliku, zanim cokolwiek zastąpi. Potem podmienia plik w jednym kroku.

    CLI goatbuilder musi wiedzieć, którego goatnetu zapytać: podaj `--site` z tym goatnetem albo ustaw `GOATBUILDER_UPDATE_URL`. Musisz być w meshu tego goatnetu. Jeśli nie jesteś, `--check` mówi, że nie może dotrzeć do serwera, i podaje adres, którego próbował. To poprawny wynik w tej lekcji.
tutor-l15-s1 = Wypisz testy, które dotyczą jednego goatnetu
tutor-l6-text =
    Wydanie przenosi nową wersję przez trzy goatnety po kolei: dev, potem preprod, potem prod.

    1. Ktoś taguje wersję na main, a CI ją buduje.
    2. dev: goatbuilder ją wdraża i uruchamia testy akceptacyjne.
    3. preprod: goatbuilder wdraża ją ponownie, ponownie uruchamia testy i zapisuje wyniki.
    4. Wersja czeka potem na preprod przez ustalony czas. Nic nie przechodzi do prod samo.
    5. prod: po tym czasie osoba uruchamia wydanie z `--from prod`. CLI goatbuilder wdraża na prod i sprawdza wszystkie trzy goatnety.

    Polecenie goatbuildera `promote status` pokazuje, gdzie jest każdy komponent. Kandydat to wersja, która jest testowana. Ogólna postać wydania to `goatbuilder release <component> --tag <tag> --from dev`. Poniższe polecenia używają jednego komponentu z tego checkoutu i przykładowego tagu. Każde ma `--dry-run`, więc nic nie zostaje otagowane ani wdrożone.

    Próbne uruchomienia wymagają `gh` i mesha. Jeśli nie jesteś administratorem repozytorium, wstaw `GOAT_RC_ROLE_CHECK=0` przed każdym poleceniem `release`.
tutor-l9-s1 = Wywołaj prawdziwy błąd z goatnetem, który nie istnieje
tutor-l9-text =
    Gdy goatbuilder odmawia, wypisuje kod. Kody zaczynające się od `GB-` pochodzą z goatbuildera, `E_` z CLI goat, a `X-` z innego narzędzia, które goatbuilder uruchomił.

    Polecenie goatbuildera `explain` przyjmuje kod albo tekst wiersza błędu i mówi ci, co się stało, dlaczego, jak to naprawić i gdzie przeczytać więcej.

    Prośba o goatnet, którego ten checkout nie deklaruje, wywołuje prawdziwy błąd do wyjaśnienia.
