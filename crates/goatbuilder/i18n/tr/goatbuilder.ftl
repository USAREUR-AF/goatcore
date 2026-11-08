### goatbuilder — tr (tr)
### source: ops/scripts/translate-ftl.py (machine)
### review: machine — see crates/goat-cli/i18n-provenance.toml

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-about = Bir goat ortamını derle ve gönder: tanımla, yakınsat, doğrula — tek bir dağıtım sözleşmesi altında
help-flag-output = Çıktı biçimi: insan için tablo; program için JSON veya YAML
help-flag-repo = Depo kökü. Varsayılan: bu ikilinin çalıştırıldığı checkout
help-flag-yes = Bildirimsel bir fiili sormadan onayla (otomasyon için)
help-flag-frozen = Checkout'u fast-forward etmeyi gerektirecek her çalıştırmayı reddet (katı C3)
help-flag-mock = Hostlar yerine bir fixture dizinine karşı çalış: registry yok, taşıma yok, deterministik (goatbuilder-conformance.md §"The fixture")
help-flag-lang = Yardım ve hata açıklamaları için dil (fr, uk, de gibi bir etiket); bir pipeline'ın okuduğu çıktı İngilizce kalır. Varsayılan: işletim sistemi yerel ayarı
help-cmd-validate = Bir yapılandırmayı sürümlü şemaya ve alanlar arası kurallara göre denetle, çevrimdışı
help-cmd-doctor = Hiçbir şey kesilmeden önce bir çalıştırmanın ihtiyaçlarını yokla: bağımlılıklar, checkout güncelliği, her ortamın taşıması ve her bileşenin playbook'unun belirli bir hedef moda çözümlendiği
help-cmd-status = Makine başına bildirilen ile kaydedilen: converged, pending veya unknown (C7, C8). Yalnızca okur. Herhangi bir makine converged değilken çıkış kodu 3
help-cmd-deploy = main'in bildirdiği pin'i tek bir ortama uygula ve kanıtla: bileşeni ve ortamın gruplarını pin'lerden, play'i ve hedef modunu, play'in şablonladığı her sırrı, taşımayı çözümle; play'i ansible-preflight üzerinden çalıştır; dağıtım defterini her hosttan geri oku ve yalnızca en yeni satır bildirilen digest'i action=deploy ile adlandırdığında bitti say (C7). Bir fixture'da (`--mock`) hostların yerine betikli converger geçer
help-cmd-release = Bir sürüm adayını main üzerindeki bir etiketten dev goatnet'in canlı doğrulamasından geçirip preprod basamağına taşı (ADR 1173); soak sonrasında `--from prod` ile mezun et, fleet: içine kat, prod'a dağıt ve her basamağı yeniden doğrula. ops/scripts/rc.sh'nin yerini adım adım alır
help-cmd-rollback = Dağıtılmış bir bileşeni, artifact türüne göre, N. en yeni ref'ine geri al; doğruluk kaynağı olarak host üzerindeki defteri oku (ADR 1060). ops/scripts/rollback.sh'nin yerini alır: çıkış 3 güvensiz olduğu için reddedildi (data, cert, forward-only, reversible:false), 4 yönlendirmeli (config, acl)
help-cmd-fastlane = Birleştirilmiş bir commit'ten her goatnet'e tek komut (ADR 1127): basamak düzeninin bilinçli istisnası. ops/scripts/fastlane.sh'nin yerini adım adım alır; pin'lenmemiş bir digest'i veya açık bir mezuniyet kümesini reddeder, dağıtımları bu ikili üzerinden çalıştırır
help-cmd-standup = Sıfırdan doğrulanmış bir siteye: standup'ın fazları doktrinin sırasıyla, her iki uçta fail-closed kapıyla, gözlemlenebilirlik hostu paralel kol olarak, kurtarma sınıfları ve goatnet'ler arası yüzeyler en sonda (ADR 1183). ops/scripts/goat-standup.sh'nin yerini adım adım alır. Bir fixture'da (`--mock`) her sınıfın her bileşeni betikli converger ile yakınsatılır ve bildirildi → doğrulandı süresi bir saatlik goatnet'in ölçüsüdür
help-cmd-join-node = Canlı bir sitede bir sınıftan bir makine daha, bildirildi → yakınsadı → doğrulandı: docs/tutorials/02-first-federated-node.md'nin yürütülen sırası, kapılarıyla numaralı fazlar olarak (erişim, anahtar, önce repo işi, baseline, kayıt öncesi ve sonrası çekirdek WireGuard, kayıt kapısı olarak çekirdek ACL düzlemi, sınıfın converge'ü, kabul)
help-cmd-federate = Egemen bir eşleşme bildirilir, yakınsatılır ve doğrulanır (ADR 1157): bizim yarımız ölçülür, paketler bant dışı değiş tokuş edilir, tek bir anlaşma oluşturulup doğrulanır, yapraklar ondan üretilir, kurallar tek yazıcıyla işlenir, uç dağıtılır, anlaşmanın seçtiği taşıma açılır, dört katmanda kabul. ops/scripts/peering-standup.sh'nin yerini adım adım alır
help-cmd-pins = İKİ çıktılı TEK çözümleme geçişi: listelenen her üyenin group_vars pins.yml'i ve listelenen her goatnet'in IntentV1 artifact'i, ops/pins/fleet-pins.yaml'dan. ops/pins/generate-pins.py'nin yerini bayt bayt alır. `--check` doğrular, yeniden üretir; sapmayı, imzasız veya kurcalanmış intent'i ve pins.yml dışında tanımlı yönetilen pin'i reddeder (çıkış 1)
help-cmd-promote = Release mezuniyetini goatnet basamakları boyunca yönlendir ve izle (ADR 1120, 1125): durum, aday aç, mezun et, küme mezun et, hotfix yolu, inceleme, hazırlık, vazgeç, denetle. ops/release/graduate.py'nin yerini adım adım alır. Burada hiçbir şey kendiliğinden mezun olmaz: bir değişiklik yalnızca çağrıldığında çalışır, ne yaptığını yazdırır ve terminal yokken --yes ister (C10, ADR 1177)
help-cmd-explain = Bir hatayı yorumla: bir kod (`GB-007`, `X-SSH-001`, `E_OFF_MESH`) veya herhangi bir hata metni — bu ikilinin kendi metni ya da taşıdığı bir aracın sözleri (ansible, ssh, op, aws, git, gh, cosign, docker, netbird). Ne olduğunu, nedenini, ne yapılacağını ve nerede daha fazla okunacağını yazdırır; `-` metni stdin'den okur

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-teardown = Bir siteyi doğrulanmış durumdan hiçliğe götürür — standup'ın aynadaki karşılığı, kayıtlar korunur (burn-down teardown satırları ekler, hiçbir şeyi silmez). Fixture'da (`--mock`) burn-down'ı ters faz sırasıyla prova eder; gerçek hostlarda sırayı yazdırır ve her faz kendi teardown play'ini bildirene kadar çalıştırmayı reddeder (docs/design/goatbuilder-speed.md)
help-cmd-tutor = Dersler, vimtutor tarzı: her ders, bir fixture'ın atılabilir kopyasına (veya salt okunur olarak bu checkout'a) karşı çalıştırılacak tam komutu belirtir, sonra bir sonraki ders açılmadan önce o komutun gerçekte ne yaptığını — çıkış kodunu ve satırlarını — denetler. `tutor` bulunduğunuz dersi gösterir ve çalıştırmalarınız şartı sağladığında dersi tamamlandı olarak işaretler; `tutor <n>`, `--list`, `--reset`
help-cmd-make = Gizli (src/fun.rs)
help-cmd-argan = Gizli (src/fun.rs)
help-cmd-bound = Gizli (crates/goat-fun)
help-cmd-secrets = Bildirilen secrets duruşu (ADR 1204): hangi depo geçerli (1password, openbao, sops, git-crypt, env-file, env), nelerin mevcut olduğu, eksik olanın nasıl sağlanacağı ve deponun kendi denetimleri. `init --env <goatnet>` sıfır-SaaS hızlı başlangıcın env dosyasını yazar: goatnet'in play'lerinin şablonladığı her değişken için güçlü rastgele bir değer, 0600, gitignore'lu, mevcut bir değeri asla değiştirmez. ops/scripts/with-secrets ile aynı modül

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-verify = Bir bileşen için her hostun en yeni deploy-ledger satırını okur ve main'in bildirdiği ref'i içerip içermediğini söyler (C7). Yalnızca okur; herhangi bir host beklemede veya bilinmiyorsa çıkış kodu 3. `deploy --verify-only` ile aynı kontrol
help-cmd-diff = Şu an main'den farkı olan nedir (C8): main'in bildirdiği ref ile her hostun en yeni ledger satırı; tek bir bileşen için ya da bileşen verilmezse ortamın tüm bileşenleri için, üye grubu başına tek geçişte okunur. Yalnızca okur; herhangi bir sapmada veya okunamayan hostta çıkış kodu 3
help-cmd-history = Deploy ledger'ı zaman çizelgesi olarak, en yeni başta, ortamın her hostunda (C8): her satır, makineyi, zamanı, eylemi (deploy veya rollback) ve sürümü içeren ayrı bir kayıttır. Kaydın önceki sürümü ve onu çalıştıran operatör `-o json` içindedir. Yalnızca okur
help-cmd-plan = Bildirimsel bir fiilin ne yapacağını inceleme için bir plan dosyasına yazar: bildirilen ref'ler, play ve hedef, ledger, main'in commit'i, taşıma yöntemi (deploy); artifact türü ve hedef ref (rollback). Hiçbir hostun durumuna dokunmaz. İncelendiği haliyle `apply` ile çalıştırın
help-cmd-apply = İncelenmiş planı birebir çalıştırır. Plan yazıldıktan sonra main'in bildirimi, pins commit'i, play veya rollback hedefi değiştiyse reddeder (çıkış kodu 1, neyin değiştiğini belirtir); taşıma yöntemi değiştiyse bildirir, reddetmez
help-cmd-adopt = Bir ortamın zaten çalıştırdığını bildirir: her hostun ledger'larını okur ve çalışan ref'i main'in bildiriminden farklı olan her bileşen için bir pins sapması önerir. Varsayılan olarak yalnızca rapor verir; --yes ile düzenlemeler inceleme için main'in yeni bir worktree'sine yazılır. Asla deploy etmez

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
tutor-not-yet = Henüz değil: { $why }
tutor-lesson-done = Ders { $n } tamamlandı: { $title }.
tutor-all-done-reset = Tüm dersler tamamlandı. `goatbuilder tutor --reset` baştan başlatır; öğretici belgeler daha ileri gider: docs/tutorials/.
tutor-all-done = Tüm dersler tamamlandı. Öğretici belgeler daha ileri gider: docs/tutorials/.
tutor-workspace = Çalışma alanı: { $path }

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-assay = Bir goatnet'in doğru kurulup kurulmadığını katman katman denetler: siteye uygulanan t
help-flag-offline = Ağdan izole site: trunk'ı çekme; checkout'u önbellekteki kopyaya göre değerlendir ve çalıştırmayı çevrimdışı olarak kaydet (C3; ops/scripts/fresh-source.sh)

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-endstate = Bir goatnet'in bildirilen son durumda olduğunu denetle: kendi bildirimlerinin çalıştığını söylediği her servis dağıtılmış, çalışıyor, sağlıklı ve görevini yapıyor; zero-trust bağlantıları kurulu; bildirilmeyen hiçbir şey çalışmıyor. Yalnızca okur; herhangi bir sapma varsa 3 koduyla çıkar
help-cmd-survey = Yazılımın her goatnet içinde ve ladder'daki goatnet'ler arasında ne kadar tutarlı olduğu, tek bir paralel geçişte okunur. Yalnızca okur

# machine-translated additions (2026-10-29) — review per i18n-provenance.toml
help-cmd-survey-long =
    Yazılımın her goatnet içinde ve merdivenin goatnet'leri arasında ne kadar tutarlı olduğu, tek bir paralel geçişte okunur. Yalnızca okur. Bir goatnet içinde: main'in bildirdiği ref, her hostun en yeni ledger satırı ve o imajdan çalıştırdığı konteynerlerle karşılaştırılır (çalışan bir konteyner; durdurulmuş olan yalnızca hiçbir şeyin çalışmadığı sıcak yedekte değerlendirilir), aynı goatnet'in birbirinden ayrılan hostları, her hosttaki goat CLI'nin kendi goatnet'inin sunduğuyla karşılaştırılması ve goat-get'in sunduğunun main ile karşılaştırılması. goatnet'ler arasında: bileşen başına bir satır, goatnet başına bir sütun, promote status'un verdiği gibi her fark için merdivenin notu. Herhangi bir sapma ya da okunamayan bir şey olduğunda çıkış 3; merdivenin bildirdiği bir fark raporlanır, başarısızlık sayılmaz.
help-cmd-update = Bu ikili dosyayı goatnet'in `get.<site>.<zone>/goatbuilder/` adresinde sunduğu sürümle değiştirir: indirme, hiçbir şeye dokunulmadan önce kendi `.sha256` dosyasına ve imzalı `SHA256SUMS` dosyasına karşı doğrulanır ve değişim atomiktir. Zaten güncel: tek satır, çıkış 0. `--check` yalnızca okur (geride kalmışsa çıkış 3). Kaynak: `GOATBUILDER_UPDATE_URL`, yoksa group_vars'tan `--site`, yoksa son güncellemenin kullandığı

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
tutor-prereq = Başlamadan önce: { $prereq }

# machine-translated additions (2026-10-30) — review per i18n-provenance.toml
help-flag-color = Tablodaki ok/FAIL simgesini ve sonucu renklendirir (auto|always|never). `auto` yalnızca terminalde renklendirir; NO_COLOR ve CLICOLOR_FORCE bunu geçersiz kılar. JSON ve YAML asla renklendirilmez
help-flag-verbose = Bir çalıştırmanın ne yaptığını — çağırdığı harici komutları ve `update` komutunun ağ aşamasını — stderr'e yazar; böylece `-o json|yaml` temiz bir belge olarak kalır. Ya da RUST_LOG=goatbuilder=debug ayarlayın
help-flag-timeout = Hızlıca başarısız olmadan önce bağlanma/erişim aşamasının sınırlanacağı saniye — erişilemeyen bir ana makinede takılabilen tek aşama (`update` komutunun sunulan sürümü çekmesi). Bir play'in kendi çalışmasını veya ikili dosya indirmesini SINIRLAMAZ (takılmaya karşı sınırlıdır). GOATBUILDER_TIMEOUT değişkenini de okur
help-cmd-completion = Kabuk tamamlama betiğini stdout'a yazar (`goatbuilder completion zsh >> ~/.zshrc`). Statik bir betiğin bilemeyeceği canlı adaylar için — bu checkout'un tanımladığı siteler, bileşenler ve goatnet'ler — bir hook'u `goatbuilder _complete` komutuna bağlayın (oluşturulan betiğin başlığına bakın)
help-cmd-docs = Komut başvurusu: `goatbuilder docs` ona yönlendirir; `--markdown` tamamını üretir, canlı komut ağacından çıkarıldığı için `--help` ile asla ayrışmaz
help-cmd-_complete = Gizli: kabuk hook'ları için canlı tamamlama adaylarını satır başına bir tane olarak üretir (bu checkout'un tanımladığı siteler, bileşenler ve goatnet'ler). Yalnızca okur, asla gürültülü başarısız olmaz — tamamlama hook'u istemi bozmamalıdır

# machine-translated additions (2026-10-31) — review per i18n-provenance.toml
help-cmd-preflight = Bir rollout öncesi hazırlık ve geri alınabilirlik raporu (ADR 1211): üç olgu — State (stateless | stateful), Rollback (pin-revert | restore-required | none) ve Scope (single | coupled) — bileşen başına bildirilir, bildirilmemişse FAIL-CLOSED çalışır; hazırlık yarısını diğer fiillerin doğruladığı kontroller oluşturur (ref çekilebilir, pins --check, secrets gate, güncel kaynak, bildirilen son durum) ve rollback olgusu ayrıca doğrulanır (pin-revert için rollback --dry-run, restore-required için saklama penceresi). `deploy` önce bunu çalıştırır; dürüstlük esaslı derecelendirilir, asla yanlış yeşil vermez. Yalnızca okur; temiz değilse çıkış kodu 3

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-l1-title = Kurulumunuzu kontrol edin
tutor-l1-s1 = Yapılandırma dosyasını kontrol edin
tutor-l2-title = Durumu okuyun: converged, pending, unknown
tutor-l2-text =
    goatbuilder `status` komutu, her makine için iki şeyi karşılaştırır: main'in o makine için bildirdiği sürümü, ve kendi kaydının çalıştığını söylediği sürümü. Hiçbir şeyi değiştirmez.

    Her makine üç durumdan birindedir:
    - converged (yakınsadı): kaydı bildirilen sürümü adlandırır. Bu tamamlanmıştır.
    - pending (beklemede): kaydı daha eski bir sürümü adlandırır, veya henüz hiçbir şeyi.
    - unknown (bilinmiyor): goatbuilder kaydını okuyamaz.

    Yalnızca converged tamamlanmış sayılır. Herhangi bir makine pending veya unknown ise, `status` 3 koduyla çıkar.

    Bu dersten itibaren, `--mock`, goatbuilder'ı gerçek makineler yerine tutor klasörünüzdeki bir pratik kopyaya yönlendirir. Orada yaptığınız hiçbir şey gerçek bir makineye ulaşmaz.

    Pratik kopyanın `dev`'ine henüz hiçbir şey dağıtılmadı. Bu yüzden dört pending makine ve bir unknown bekleyin. Converged makineleri 3. derste göreceksiniz.
tutor-l2-s1 = dev'in durumunu okuyun
tutor-l3-title = Dağıtım: planlayın, çalıştırın, kanıtlayın
tutor-l3-text =
    Dağıtım, bildirilen sürümü her makineye koyar, sonra ulaştığını kanıtlamak için her makinenin kaydını okur. Bir dağıtım, yalnızca her makinenin kaydı o sürümü adlandırdığında tamamlanmış sayılır. Hatasız çalışan bir dağıtım yeterli değildir.

    `portal`'ı pratik kopyanın `preprod`'una dağıtacaksınız. `--dry-run` ne olacağını gösterir ve hiçbir şeyi değiştirmez. `--ack`, kimin yaptığını kaydeder; komut adınızı git'ten doldurur. Dağıtımdan sonra, `status`, her iki makineyi de `ok` ve converged olarak göstermelidir.
tutor-l3-s1 = Dağıtımı planlayın
tutor-l3-s2 = portal'ı preprod'a dağıtın
tutor-l3-s3 = Her iki makinenin de converged olduğunu kontrol edin
tutor-l4-title = Kayma ve geçmiş
tutor-l4-text =
    İki komut, bir değişiklikten önce ve sonra bir goatnet'i anlamanıza yardımcı olur. Her ikisi de yalnızca okur.
    - goatbuilder `diff` komutu, kaydı bildirilen sürümden farklı olan her makineyi listeler. Bir farka kayma denir.
    - goatbuilder `history` komutu, bir bileşenin her makinede sahip olduğu her sürümü, en yeniden başlayarak listeler.

    `dev`'de, her makine geridedir. `portal`'ın `preprod` üzerindeki geçmişi artık 3. dersteki dağıtımınızı içeriyor.
tutor-l4-s1 = dev'deki kaymayı listeleyin
tutor-l4-s2 = portal'ın preprod'daki geçmişini okuyun
tutor-l5-title = Geri alın, ve güvensiz bir geri almanın reddedildiğini görün
tutor-l5-text =
    Bir geri alma, daha eski bir sürümü bildirir ve onu başka her sürüm gibi dağıtır. goatbuilder CLI'si sonra ulaştığını kanıtlamak için her makinenin kaydını okur.

    Bazı değişiklikler bu şekilde geri alınamaz. Daha eski bir sürümü dağıtmak, bir bileşenin yazdığı veriyi, bir sertifikayı veya yalnızca ileri giden bir değişikliği geri alamaz. goatbuilder CLI'si bunları 3 koduyla reddeder ve yerine ne yapılacağını söyler.

    Pratik kopyanın `db-schema` bileşeni veri tuttuğu için, goatbuilder onu geri almayı reddeder. Reddetme, beklenen sonuçtur.
tutor-l5-s1 = Geri almayı planlayın
tutor-l5-s2 = portal'ı bir sürüm geri alın
tutor-l5-s3 = goatbuilder'ın reddettiği db-schema'yı geri almayı deneyin
tutor-l6-title = Yeni bir sürümü yayınlayın: dev, sonra preprod, sonra prod
tutor-l6-s1 = Her bileşenin nerede olduğunu görün
tutor-l6-s2 = Yayının dev'den deneme çalıştırmasını yapın
tutor-l6-s3 = prod adımının deneme çalıştırmasını yapın
tutor-l7-title = Fastlane, ve ne zaman kullanılmaması gerektiği
tutor-l7-text =
    Fastlane, bir sürümü tüm goatnet'lere aynı anda koyar. preprod'daki beklemeyi ve yayın kayıtlarını atlar.

    Onu yalnızca önceki sürüme dönerek geri alabileceğiniz acil bir düzeltme için kullanın. Onu asla bir veritabanı değişikliği, bir kimlik veya erişim değişikliği, veya bu şekilde geri alamayacağınız herhangi bir şey için kullanmayın.

    Fastlane, CI'nin derlemediği bir sürümü, ve halen sürmekte olan bir yayının parçası olan bir bileşeni reddeder.

    Genel biçim `goatbuilder fastlane <component> --ref <version> --reason <why>`'dır. Aşağıdaki örneği çalıştırın. İçinde `--dry-run` var, bu yüzden hiçbir şey değişmez. `gh`'a gerek duyar.
tutor-l7-s1 = Bir fastlane'in deneme çalıştırmasını yapın
tutor-l8-title = Pins: kaymayı bulun ve doğru şekilde düzeltin
tutor-l8-text =
    Bir dosya, her goatnet'in hangi sürümü çalıştırdığını belirler: `ops/pins/fleet-pins.yaml`. goatbuilder CLI'si diğer sürüm dosyalarını ondan üretir: `pins.yml` dosyalarını ve imzalı intent dosyalarını.

    goatbuilder `pins --check` komutu, üretilen dosyaları `fleet-pins.yaml` ile karşılaştırır ve farklı olan her dosyayı reddeder. goatbuilder `pins` komutu onları yeniden üretir.

    Tutor klasörünüzde, birinin bir `pins.yml` dosyasını elle düzenlediği bir depo kopyası var. Elle yapılan bir düzenlemeyi başka bir elle düzenlemeyle asla düzeltmeyin. `fleet-pins.yaml`'ı değiştirin, sonra yeniden üretin.
tutor-l8-s1 = Bu checkout'u kontrol edin
tutor-l8-s2 = Kopyayı kontrol edin ve elle yapılan düzenlemeyi bulun
tutor-l8-s3 = Kopyanın dosyalarını yeniden üretin
tutor-l8-s4 = Kopyayı yeniden kontrol edin
tutor-l9-title = Bir hatayı açıklayın
tutor-l9-s2 = Hatayı koduyla açıklayın
tutor-l9-s3 = Onu metniyle açıklayın
tutor-l10-title = Öğretici 1: yeni bir goatnet'in ilk sitesini kurun
tutor-l10-text =
    Bu ders bir pratik kopyada bütün bir goatnet kurar. Örnek goatnet `zagros`'dir. Onun ilk sitesi, `alpha`, iki çekirdek makineye, bir izleme makinesine ve bir kurtarma makinesine sahiptir.

    goatbuilder `standup` komutu, her makineyi sabit bir sırayla kurar ve her kaydı kontrol eder. Son satırı sitenin tamamlandığını söyler. Gerçek makinelerde, o satır ne kadar sürdüğünü de gösterir.

    Öğreticinin tamamı `docs/tutorials/01-new-goatnet.md`'dir.
tutor-l10-s1 = İlk siteyi, alpha'yı, kurun
tutor-l10-s2 = Sonucu kontrol edin
tutor-l11-title = Öğretici 2: ikinci bir site ekleyin
tutor-l11-text =
    Bu ders, 1. öğreticinin bittiği yerden başlar. Yeni bir pratik kopyada, `zagros-2`, ikinci bir site, `beta`, ekler.

    İkinci bir site, kullanıcı hesaplarını ilk siteden kopyalar. Onun standup'ı da kopyalamanın her iki yönde çalışması için ilk siteyi bir kez değiştirir. `alpha/core` adlı bir adım bunu yapar.

    Öğreticinin tamamı `docs/tutorials/02-first-federated-node.md`'dir.
tutor-l11-s1 = İkinci siteyi, beta'yı, kurun
tutor-l11-s2 = Her iki siteyi de kontrol edin
tutor-l12-s1 = Bir makine eklemeyi planlayın
tutor-l13-s1 = Başka bir goatnet'e bağlanmayı planlayın
tutor-l14-title = goatbuilder'ı güncel tutun
tutor-l14-s1 = Daha yeni bir goatbuilder'ın var olup olmadığını sorun
tutor-l15-title = Assay: bir goatnet'in nasıl kurulduğunu kontrol edin
tutor-l15-text =
    goatbuilder `assay` komutu, kabul testlerini kullanarak bir goatnet'in bir seferde bir katman olmak üzere doğru kurulduğunu kontrol eder. Her test üç sonuçtan birini alır:
    - PASS: test geçti.
    - FAIL: goatnet'te bir sorun var.
    - UNKNOWN: hiçbir şey teste cevap veremedi.

    Yalnızca PASS sayılır. Herhangi bir FAIL veya UNKNOWN, çıkış kodunu 3 yapar.

    `--plan`, bir goatnet'e uygulanan testleri, katmana göre listeler ve hiçbir makineye dokunmaz. Gerçek bir assay'in neyi kapsayacağını görmek için kullanın. Genel biçim `goatbuilder assay <goatnet> --plan`'dır.
tutor-l16-title = Survey: sürümler tutarlı mı?
tutor-l16-text =
    goatbuilder `survey` komutu, aynı sürümlerin gerekli yerlerde çalışıp çalışmadığını tek bir geçişte kontrol eder:
    - Her goatnet içinde: her makine bildirilen sürümü çalıştırıyor mu, ve eşleşmesi gereken makineler gerçekten eşleşiyor mu?
    - dev, preprod ve prod arasında: bileşen başına bir satır, goatnet başına bir sütun.

    Yayın sürecinin beklediği bir fark raporlanır, ama bir hata değildir. Bir örnek, dev'de olan ama henüz prod'da olmayan yeni bir sürümdür. Hiçbir şeyin bildirmediği bir fark `difference not declared` olarak okunur. goatbuilder'ın okuyamadığı bir makine unknown'dır, asla tutarlı değildir.

    Onu pratik kopya üzerinde çalıştırın. Onun `dev`'i kasıtlı olarak geridedir, bu yüzden bulgular ve çıkış kodu 3 bekleyin.
tutor-l16-s1 = Pratik kopya üzerinde survey çalıştırın
tutor-l17-title = Endstate: bir goatnet tam olarak bildirdiğini mi çalıştırıyor?
tutor-l17-text =
    goatbuilder `endstate` komutu, bir goatnet hakkında iki şeyi kontrol eder:
    1. Bildirdiği her servis dağıtılmış, çalışıyor, sağlıklı ve doğru şekilde bağlı mı.
    2. Bildirmediği hiçbir şey çalışmıyor mu. Bildirilmemiş bir servise yetim denir.

    goatbuilder CLI'si, bir sitenin ne çalıştırması gerektiğini ayrı bir listeden değil, sitenin kendi ayarlarından çıkarır.

    Okuyacak hiçbir şeyi olmayan bir kontrol UNOBSERVED olarak işaretlenir. Bu dürüst bir cevaptır, bir başarısızlık değildir. Bir FAIL veya bir yetim, çıkış kodunu 3 yapar.

    Genel biçim `goatbuilder endstate <goatnet>`'dir. Aşağıdaki komut, kaydedilmiş veriden örnek bir siteyi kontrol eder, bu yüzden hiçbir makineye dokunmaz.
tutor-l17-s1 = Örnek siteyi kontrol edin
tutor-l18-title = Bonus: meraklı olun
tutor-l18-text =
    Bu ders isteğe bağlıdır. Aşağıdaki komutu çalıştırın ve tutor tamamlanmış olur.

    goatbuilder CLI'sinin birkaç küçük gizli özelliği vardır. Bunlar yardımda veya bu derslerde yoktur. Bazıları, eski Unix kullanıcılarının sadece ne olacağını görmek için yeni bir programa sık sık yazdığı kelimelere cevap verir. Deneyeceğiniz şeylerin çoğu hiçbir şey yapmaz.

    Bu ders, onların ne olduğunu söylemez. Onları bulmak, asıl amaçtır.

    Aşağıdaki komut, goatbuilder'ın açıklayabileceği her hatayı listeler. Keşfe başlamak için iyi bir yerdir.
tutor-l18-s1 = goatbuilder'ın açıklayabileceği her hatayı listeleyin
tutor-heading = Ders { $n } / { $total } · { $title }
tutor-setup = Önce, bunu bu terminalde bir kez çalıştırın:
tutor-not-recording = Bu terminal çalıştırmalarınızı kaydetmiyor, bu yüzden tutor adımlarınızı işaretleyemez. Bunu bir kez çalıştırın, sonra adımları çalıştırın:
tutor-type = Bu adımları terminalinizde yapın ({ $met } / { $total } tamamlandı):
tutor-rerun = Her adımdan sonra, `goatbuilder tutor`'u tekrar çalıştırın. Ne yaptığınızı kontrol eder ve adımı işaretler.

# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-welcome = GOATBUILDER Tutor'a hoş geldiniz
tutor-bar-progress = İlerleme
tutor-bar-done = { $done } / { $total } ders tamamlandı
tutor-after-step = Adım { $met } / { $total } tamamlandı. Sonraki:
tutor-after-lesson = Ders { $n } tamamlandı. Ders { $next }'i başlatmak için `goatbuilder tutor`'u çalıştırın. Yeniden okumak için: `goatbuilder tutor --lesson { $n }`.
tutor-after-final = Ders { $n } tamamlandı, tutor da öyle. Tamamlandı olarak işaretlendiğini görmek için `goatbuilder tutor`'u çalıştırın. Yeniden okumak için: `goatbuilder tutor --lesson { $n }`.

tutor-review = Bu dersi tamamladınız, bu nedenle bu sayfa yalnızca okumak içindir. Bulunduğunuz yere geri dönmek için `goatbuilder tutor`'u çalıştırın.
tutor-other-lessons = Bir dersi yeniden okumak için: `goatbuilder tutor --lesson 1`. Hepsini görmek için: `goatbuilder tutor --list`.
tutor-l12-title = Çalışan bir siteye makine ekleyin
tutor-l12-text =
    goatbuilder `join-node` komutu, zaten çalışan bir siteye bir makine daha ekler. Önce bir plan yazdırır, her faz için bir satır. Her fazın bir türü vardır:
    - tool: goatbuilder bir script çalıştırır.
    - ansible: goatbuilder bir playbook çalıştırır.
    - gate: bir sonraki fazdan önce geçmesi gereken bir kontrol.
    - manual: bunu kendiniz yaparsınız, ve çalıştırma duraklar.

    Aşağıdaki komutun `--dry-run`'ı vardır. Bu checkout'u okur ve hiçbir şeyi değiştirmez. Tablonun altında, elle ne yaptığınızı okuyun.
tutor-l13-title = Başka bir kuruluşun goatnet'ine bağlanın
tutor-l13-text =
    goatbuilder `federate` komutu, goatnet'inizi başka bir kuruluşun çalıştırdığı bir goatnet'e bağlar. Her taraf bir anlaşmanın kendi yarısını oluşturur. Yarıları değiş tokuş edersiniz ve her taraf onları aynı anlaşmada birleştirir.

    Diğer tarafın anahtarını ayrı bir yolla doğrulayın, örneğin güvendiğiniz bir aramayla. Onların paketinin içinde gelen bir anahtar hiçbir şey kanıtlamaz.

    Aşağıdaki komutun `--dry-run`'ı vardır, bu yüzden hiçbir şeyi değiştirmez. Gerçek bir çalıştırma faz 4'te durur, çünkü o fazı elle yaparsınız.
# machine-translated additions (2026-11-01) — review per i18n-provenance.toml
tutor-where = Bu komut ders { $n }'in parçası değil. Bulunduğunuz yeri görmek için `goatbuilder tutor`'u çalıştırın.
tutor-l1-s2 = Bir goatnet hakkında doctor'a sorun
tutor-l1-text =
    goatbuilder CLI'ı GOAT yazılımını derler, dağıtır ve yayınlar. Kullanmadan önce kurulumunuzu iki komutla kontrol edin:
    - goatbuilder `validate` komutu yapılandırma dosyasını, yani `ops/goatbuilder/goatbuilder.yaml` dosyasını kontrol eder. Yalnızca bu checkout'u okur ve ağa ihtiyaç duymaz.
    - goatbuilder `doctor` komutu gerçek bir dağıtımın neye ihtiyaç duyduğunu kontrol eder: araçlarınızı, oturum açmalarınızı, bu checkout'un main ile eşleşip eşleşmediğini ve goatbuilder'ın her goatnet'e nasıl ulaştığını. Bir goatnet, bir GOAT ortamıdır.

    goatbuilder `doctor` komutu kaç kontrolün geçtiğini, ardından her sorunu düzeltmesiyle birlikte yazdırır. Bu derste bir sorun bir hata değil, bilgidir. Düzeltmeleri okuyun, ancak şimdi uygulamanız gerekmez.

    Çoğu ders oturum açmayı gerektirmez. 2 ile 5 arası, 10, 11 ve 16. dersler bir alıştırma kopyası kullanır. 8, 9, 12, 13, 15, 17 ve 18. dersler yalnızca bu checkout'u okur. 6. ders `gh` ve mesh gerektirir, 7. ders `gh` gerektirir ve 14. ders mesh gerektirir. Hiçbir ders 1Password gerektirmez.
tutor-l14-text =
    goatbuilder CLI'ı, goatnet'inizin `get.<site>.<zone>/goatbuilder/` adresinde sunduğu kopyadan kendini güncelleyebilir.

    goatbuilder `update --check` komutu yalnızca okur. Sürümünüzü sunulan sürümle karşılaştırır ve hiçbir şeyi değiştirmez. Çıkış kodu 0, güncel olduğunuz anlamına gelir. Çıkış kodu 3, daha yeni bir sürüm olduğu anlamına gelir.

    `--check` olmadan yapılan gerçek bir güncelleme, herhangi bir şeyi değiştirmeden önce indirilen dosyanın sağlama toplamını ve imzasını doğrular. Ardından dosyayı tek adımda değiştirir.

    goatbuilder CLI'ının hangi goatnet'e soracağını bilmesi gerekir: o goatnet ile `--site` verin veya `GOATBUILDER_UPDATE_URL` değişkenini ayarlayın. O goatnet'in mesh'inde olmanız gerekir. Değilseniz, `--check` sunucuya ulaşamadığını söyler ve denediği adresi belirtir. Bu ders için bu doğru bir sonuçtur.
tutor-l15-s1 = Bir goatnet için geçerli olan testleri listeleyin
tutor-l6-text =
    Bir yayın, yeni bir sürümü sırayla üç goatnet'ten geçirir: dev, sonra preprod, sonra prod.

    1. Biri main üzerinde sürümü etiketler ve CI onu derler.
    2. dev: goatbuilder onu dağıtır ve kabul testlerini çalıştırır.
    3. preprod: goatbuilder onu yeniden dağıtır, testleri yeniden çalıştırır ve sonuçları kaydeder.
    4. Sürüm daha sonra belirli bir süre preprod'da bekler. Hiçbir şey prod'a kendiliğinden geçmez.
    5. prod: bu beklemeden sonra bir kişi yayını `--from prod` ile çalıştırır. goatbuilder CLI'ı prod'a dağıtır ve üç goatnet'in hepsini kontrol eder.

    goatbuilder `promote status` komutu her bileşenin nerede olduğunu gösterir. Aday, test edilmekte olan bir sürümdür. Bir yayının genel biçimi `goatbuilder release <component> --tag <tag> --from dev` şeklindedir. Aşağıdaki komutlar bu checkout'un bir bileşenini ve örnek bir etiketi kullanır. Her birinde `--dry-run` vardır, bu yüzden hiçbir şey etiketlenmez veya dağıtılmaz.

    Deneme çalıştırmaları `gh` ve mesh gerektirir. Deponun yöneticisi değilseniz, her `release` komutunun önüne `GOAT_RC_ROLE_CHECK=0` koyun.
tutor-l9-s1 = Var olmayan bir goatnet ile gerçek bir hata oluşturun
tutor-l9-text =
    goatbuilder reddettiğinde bir kod yazdırır. `GB-` ile başlayan kodlar goatbuilder'dan, `E_` ile başlayanlar goat CLI'ından, `X-` ile başlayanlar ise goatbuilder'ın çalıştırdığı başka bir araçtan gelir.

    goatbuilder `explain` komutu bir kodu veya bir hata satırının metnini alır ve size ne olduğunu, nedenini, nasıl düzelteceğinizi ve nerede daha fazla okuyacağınızı söyler.

    Bu checkout'un bildirmediği bir goatnet'i istemek, açıklanacak gerçek bir hata oluşturur.
