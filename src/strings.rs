//! 界面文案表（F3 起）。
//!
//! # 为什么不是 gettext
//!
//! gettext 的查找发生在**运行期**：漏翻译的地方不会编译失败，只会在界面上露出
//! 一句原语言。这个项目宁愿要"编译期就过不去"。所以这里是一张**编译期**的表：
//! 每条文案都是 `Strings` 的一个命名字段，中英两列写在同一个宏臂里。
//!
//! # 与路标里写法的差异（有意为之）
//!
//! `FUTURE_ROADMAP.md` §4 写的是"`struct Strings` + 两个 `static`"，那样新增字段时
//! 两个实例都要补齐，**漏一个**是编译错误。这里用 [`strings!`] 把中英两列写成**同一条
//! 宏臂的两个值**：漏翻译不再是"编译错误"，而是**根本写不出来**——比原方案更强，
//! 且中英对照就并排放在一行，review 时不必来回翻两处。
//!
//! # 哪些文案**不**翻译
//!
//! 只翻译 **UI 控件文案**。接收区里由程序写出的提示行（"串口已关闭"、"发送失败: …"、
//! "已导出到: …"）与实时日志的内容**保持固定中文**，理由见 `FUTURE_ROADMAP.md` §4：
//! 这些文字会同步写进实时日志文件与导出文件，若随界面语言变化，同一台设备在两个
//! 语言设置下产出的两份日志就不可比、也不能被 grep 脚本处理。
//!
//! 因此本文件里**没有**那几条提示行的字段。看到 `receive.rs` / `app.rs` / `log_file.rs`
//! 里仍然硬编码中文的 `push_receive_note("串口已关闭\n")` 时，那是决策，不是漏改。
//!
//! # 带占位符的文案
//!
//! `format!` 的格式串**必须是字面量**（变量会报 `format argument must be a string
//! literal`），所以带参数的文案不能直接 `format!(s.xxx, arg)`。改用 [`fill`]：
//! 模板里的 `{}` 按顺序被实参替换。这条限制正是 `fill` 存在的原因。

use crate::app::Language;
use crate::receive::TimestampPrefix;

/// N 列的文案表。
///
/// # 形状
///
/// ```text
/// strings! {
///     // 第一行声明语言列的顺序；列数即语言种数，名称须是大写标识符
///     // （宏生成 `static ZH` 等符号时直接用这个名字，无法转换大小写）。
///     langs: ZH, EN, ES;
///     // 之后每条文案一行：字段名 + 各语言文案，个数必须与 langs 相等。
///     menu_file: "文件", "File", "Archivo";
///     ...
/// }
/// ```
///
/// **列数即语言的种数**：每条文案后面的字面量个数必须与 `langs:` 列表长度相等，
/// 少一个/多一个都是编译错误（宏为某语言取第 i 列时取不到就报 `unexpected end`）。
///
/// # 为什么是"宏臂写多列"而不是"多个 static"
///
/// 早先（F3）这里**写死两列**：`$zh:literal, $en:literal`，于是"加一种语言"
/// 必须先改宏。现在每条文案是 `$( $text:literal ),+`，加语言 = **纯加数据**：
/// 在 `langs:` 里多写一个名字、在每条文案后面多写一个字面量。
/// 漏掉任何一条都会在编译期暴露。
///
/// # 生成的符号
///
/// 对 `langs: ZH, EN, ES`，宏生成：
/// - `struct Strings`：每条文案一个字段；
/// - `static ZH` / `static EN` / `static ES`：各语言一张表；
/// - `#[cfg(test)] const LANGS: &[&str]`：`["ZH", "EN", "ES"]`，列顺序表（仅测试用）；
/// - `#[cfg(test)] const ALL: &[(&str, &[&str])]`：`(字段名, 各列)`。
///
/// # 转置是怎么做到的
///
/// 宏无法在"同一层重复"里对多列列表做 zipper，所以这里分两步：
/// ① `@collect` 把每条文案的 `字段: 列1, 列2, …` 原样收进一个"行列表"；
/// ② `@emit_langs` 对 `langs` 里的每个名字 `L`，用 `@peel` 从**每一行**里
///    剥掉前 k 列、取出第 k+1 列，拼成 `static L`。
macro_rules! strings {
    (
        langs: $($lang:ident),+ $(,)? ;
        $( $field:ident: $($text:literal),+ ; )*
    ) => {
        /// 一个语言的界面文案。字段名即用途；取值只从各语言表（[`ZH`]/[`EN`]…）里取。
        pub(crate) struct Strings {
            $( pub(crate) $field: &'static str, )*
        }

        /// 列顺序表：第 i 项是第 i 列的语言名（与生成的 `static` 同名）。
        ///
        /// [`crate::app::Language::ALL`] 的顺序**必须与它一致**——`Language::strings()`
        /// 按下标取对应的表，错位会让界面显示另一种语言。
        /// `tests::langs_table_matches_language_order` 把两者钉在一起。
        ///
        /// 运行期没有任何地方读它，只有测试用它做顺序校验，所以 `#[cfg(test)]`
        /// （否则 release 构建会报 dead_code）。
        #[cfg(test)]
        pub(crate) const LANGS: &[&str] = &[ $( stringify!($lang), )+ ];

        strings!(@emit_langs $($lang),+ ; $( $field: $($text),+ ; )* );

        /// 每条文案的 `(字段名, 各列文案)`，列顺序与 [`LANGS`] 一致。
        /// **仅供测试**遍历用（运行期没有任何地方读它）。
        #[cfg(test)]
        pub(crate) const ALL: &[(&str, &[&str])] =
            &[ $( (stringify!($field), &[$($text),+]), )* ];
    };

    // ── 逐语言生成 `static`。
    //
    // 关键：`macro_rules!` **不能拼接 token**（写不出 `(m m) + m -> (m m m)`），
    // 也不能在匹配时做算术（`[$n - 1]` 不成立）。所以"第 k 列"不能靠累加计数器，
    // 只能**边走边剥**：每处理一种语言，就从每行的字面量列表里**取走第一个**，
    // 剩下的尾巴交给下一种语言。尾巴天然是"剩余列"，不需要算下标。
    // 还有后续语言：生成当前这张表，把每行的尾巴交给下一个语言。
    (@emit_langs $lang:ident, $($rest:ident),+ ;
        $( $field:ident: $first:literal $(, $tail:literal)* ; )*
    ) => {
        /// 一种语言的文案表（表名 = `langs:` 里该语言的名字）。
        pub(crate) static $lang: Strings = Strings {
            $( $field: $first, )*
        };
        strings!(@emit_langs $($rest),+ ; $( $field: $($tail),* ; )* );
    };
    // 最后一个语言（`$rest` 为空，不能再写 `$($rest),+`——那要求至少一个）。
    (@emit_langs $lang:ident ;
        $( $field:ident: $first:literal $(, $tail:literal)* ; )*
    ) => {
        /// 一种语言的文案表（表名 = `langs:` 里该语言的名字）。
        pub(crate) static $lang: Strings = Strings {
            $( $field: $first, )*
        };
        // 列数必须与语言数相等：若某行还剩列，这里会编译期报错。
        $( strings!(@no_leftovers $field: $($tail),*); )*
    };
    // 尾巴为空 = 正常；尾巴非空 = 列数不匹配，编译期报错。
    (@no_leftovers $field:ident:) => {};
    (@no_leftovers $field:ident: $($leftover:literal),+) => {
        compile_error!(concat!(
            "strings! 表里 `", stringify!($field),
            "` 这一条的列数比 `langs:` 声明的语言多——请补齐或删掉多余列"
        ));
    };
}

strings! {
    // 语言列顺序。**必须与 `Language::ALL` 顺序一致**（`tests::langs_table_matches_
    // language_order` 钉住），也是 `Language::strings()` 按下标取表的依据。
    // 名字须大写标识符：宏生成的 `static` 直接用它。
    // 顺序 = 风险从低到高（见 I18N_TARGET_LANGUAGES.md §0）。
    langs: ZH, EN, ZH_HANT, JA, ES, IT, PT, ID, FR, DE, KO, VI, TR, RU, PL;

    // —— app.rs：顶栏按钮 ——
    sidebar_toggle: "☰ 侧边栏", "☰ Sidebar",
        "☰ 側邊欄",
        "☰ サイドバー",
        "☰ Barra lateral",
        "☰ Barra laterale",
        "☰ Barra lateral",
        "☰ Bilah samping",
        "☰ Barre latérale",
        "☰ Seitenleiste",
        "☰ 사이드바",
        "☰ Thanh bên",
        "☰ Kenar çubuğu",
        "☰ Боковая панель",
        "☰ Pasek boczny";

    // —— app.rs：接收设置行 ——
    receive_settings: "接收设置:", "Receive:",
        "接收設定:",
        "受信設定:",
        "Recepción:",
        "Ricezione:",
        "Recepção:",
        "Penerimaan:",
        "Réception :",
        "Empfang:",
        "수신 설정:",
        "Nhận:",
        "Alım:",
        "Приём:",
        "Odbiór:";
    filter_toggle: "筛选", "Filter",
        "篩選",
        "絞り込み",
        "Filtrar",
        "Filtra",
        "Filtrar",
        "Filter",
        "Filtrer",
        "Filtern",
        "필터",
        "Lọc",
        "Filtrele",
        "Фильтр",
        "Filtruj";
    filter_hint: "只显示含此内容的行", "Show only lines containing this",
        "只顯示含此內容的行",
        "この文字列を含む行だけ表示",
        "Mostrar solo las líneas que contengan esto",
        "Mostra solo le righe che contengono questo",
        "Mostrar apenas as linhas que contêm isto",
        "Tampilkan hanya baris yang memuat ini",
        "Afficher seulement les lignes contenant ceci",
        "Nur Zeilen mit diesem Inhalt anzeigen",
        "이 내용이 포함된 줄만 표시",
        "Chỉ hiện các dòng chứa nội dung này",
        "Yalnızca bunu içeren satırları göster",
        "Показывать только строки с этим текстом",
        "Pokaż tylko wiersze zawierające to";
    filter_clear: "清空筛选", "Clear filter",
        "清空篩選",
        "絞り込みを解除",
        "Quitar filtro",
        "Rimuovi filtro",
        "Limpar filtro",
        "Hapus filter",
        "Effacer le filtre",
        "Filter löschen",
        "필터 지우기",
        "Xóa bộ lọc",
        "Filtreyi temizle",
        "Сбросить фильтр",
        "Wyczyść filtr";
    filter_count: "命中 {} / {} 行", "Matching {} of {} lines",
        "命中 {} / {} 行",
        "{} / {} 行が該当",
        "{} de {} líneas coinciden",
        "{} di {} righe corrispondono",
        "{} de {} linhas correspondem",
        "{} dari {} baris cocok",
        "{} sur {} lignes correspondent",
        "{} von {} Zeilen passen",
        "{} / {} 줄 일치",
        "{} trên {} dòng khớp",
        "{} / {} satır eşleşti",
        "Совпало {} из {} строк",
        "{} z {} wierszy pasuje";
    filter_note: "子串匹配、区分大小写；只保留含该子串的整行。导出与实时日志文件仍是完整原文",
        "Substring match, case-sensitive; keeps the whole line when it contains it. Export and the live log file still keep the full text",
        "子字串比對、區分大小寫；只保留含該子串的整行。匯出與即時日誌檔仍是完整原文",
        "部分一致・大文字小文字を区別。含む行は丸ごと表示します。エクスポートとリアルタイムログは常に全文です",
        "Coincidencia de subcadena, distingue mayúsculas; conserva la línea completa si la contiene. La exportación y el archivo de registro en vivo siguen con el texto completo",
        "Corrispondenza di sottostringa, sensibile alle maiuscole; mantiene l'intera riga se la contiene. Esportazione e file di log live restano completi",
        "Correspondência de subcadeia, diferencia maiúsculas; mantém a linha inteira se a contiver. A exportação e o arquivo de log ao vivo continuam completos",
        "Pencocokan substring, peka huruf besar-kecil; baris ditampilkan utuh jika memuatnya. Ekspor dan berkas log langsung tetap lengkap",
        "Correspondance de sous-chaîne, sensible à la casse ; la ligne entière est conservée si elle la contient. L'export et le journal en direct restent complets",
        "Teilzeichenfolge, Groß-/Kleinschreibung wird beachtet; die ganze Zeile bleibt, wenn sie sie enthält. Export und Live-Log bleiben vollständig",
        "부분 문자열 일치, 대소문자 구분. 포함된 줄은 통째로 표시합니다. 내보내기와 실시간 로그 파일은 항상 전체입니다",
        "Khớp chuỗi con, phân biệt chữ hoa chữ thường; giữ nguyên cả dòng nếu có chứa. Xuất tệp và tệp nhật ký trực tiếp vẫn đầy đủ",
        "Alt dize eşleşmesi, büyük-küçük harfe duyarlı; içeren satır tümüyle gösterilir. Dışa aktarma ve canlı günlük dosyası tam kalır",
        "Поиск подстроки с учётом регистра; строка показывается целиком. Экспорт и файл журнала остаются полными",
        "Dopasowanie podciągu, z rozróżnianiem wielkości liter; wiersz jest pokazywany w całości. Eksport i plik dziennika na żywo pozostają pełne";
    color: "颜色", "Color",
        "顏色",
        "色",
        "Color",
        "Colore",
        "Cor",
        "Warna",
        "Couleur",
        "Farbe",
        "색상",
        "Màu",
        "Renk",
        "Цвет",
        "Kolor";
    byte_view: "字节查看", "Byte view",
        "位元組檢視",
        "バイト表示",
        "Vista de bytes",
        "Vista byte",
        "Vista de bytes",
        "Tampilan bita",
        "Vue octets",
        "Byte-Ansicht",
        "바이트 보기",
        "Xem byte",
        "Bayt görünümü",
        "Просмотр байтов",
        "Widok bajtów";

    // —— app.rs：发送区 ——
    send_area: "发送区:", "Send:",
        "傳送區:",
        "送信エリア:",
        "Envío:",
        "Invio:",
        "Envio:",
        "Kirim:",
        "Envoi :",
        "Senden:",
        "송신 영역:",
        "Gửi:",
        "Gönderme alanı:",
        "Отправка:",
        "Wysyłanie:";
    format: "格式", "Format",
        "格式",
        "形式",
        "Formato",
        "Formato",
        "Formato",
        "Format",
        "Format",
        "Format",
        "형식",
        "Định dạng",
        "Biçim",
        "Формат",
        "Format";
    line_ending: "行尾", "Line ending",
        "行尾",
        "改行コード",
        "Fin de línea",
        "Fine riga",
        "Fim de linha",
        "Akhir baris",
        "Fin de ligne",
        "Zeilenende",
        "줄 끝",
        "Kết thúc dòng",
        "Satır sonu",
        "Конец строки",
        "Koniec wiersza";
    repeat_send: "定时发送", "Repeat send",
        "定時傳送",
        "定期送信",
        "Envío periódico",
        "Invio periodico",
        "Envio periódico",
        "Kirim berkala",
        "Envoi périodique",
        "Wiederholtes Senden",
        "반복 전송",
        "Gửi định kỳ",
        "Yineli gönder",
        "Повторная отправка",
        "Wysyłanie cykliczne";
    preview_too_long: "输入过长，已停止实时校验", "Input too long — live validation paused",
        "輸入過長，已停止即時校驗",
        "入力が長すぎるため、リアルタイム検証を停止しました",
        "Entrada demasiado larga: validación en vivo en pausa",
        "Input troppo lungo: convalida in tempo reale sospesa",
        "Entrada demasiado longa: validação em tempo real em pausa",
        "Masukan terlalu panjang — validasi langsung dijeda",
        "Entrée trop longue : validation en direct suspendue",
        "Eingabe zu lang — Live-Prüfung angehalten",
        "입력이 너무 깁니다 — 실시간 검사 일시 중지",
        "Nhập quá dài — tạm dừng kiểm tra trực tiếp",
        "Girdi çok uzun — canlı doğrulama duraklatıldı",
        "Ввод слишком длинный — проверка в реальном времени приостановлена",
        "Dane wejściowe są za długie — walidacja na bieżąco wstrzymana";
    will_send_bytes: "将发送 {} 字节", "Will send {} bytes",
        "將傳送 {} 位元組",
        "{} バイト送信します",
        "Se enviarán {} bytes",
        "Verranno inviati {} byte",
        "Serão enviados {} bytes",
        "Akan mengirim {} bita",
        "{} octets seront envoyés",
        "{} Bytes werden gesendet",
        "{} 바이트 전송 예정",
        "Sẽ gửi {} byte",
        "{} bayt gönderilecek",
        "Будет отправлено {} байт",
        "Zostanie wysłane {} bajtów";
    unmappable_in_encoding:
        "有字符在 {} 里无法表示，将被替换",
        "Some characters cannot be represented in {} and will be replaced",
        "有字元在 {} 裡無法表示，將被替換",
        "一部の文字は {} で表現できず、置換されます",
        "Algunos caracteres no se pueden representar en {} y se reemplazarán",
        "Alcuni caratteri non sono rappresentabili in {} e verranno sostituiti",
        "Alguns caracteres não podem ser representados em {} e serão substituídos",
        "Beberapa karakter tidak dapat diwakili dalam {} dan akan diganti",
        "Certains caractères ne peuvent pas être représentés en {} et seront remplacés",
        "Einige Zeichen lassen sich in {} nicht darstellen und werden ersetzt",
        "일부 문자는 {}에서 표현할 수 없어 대체됩니다",
        "Một số ký tự không thể biểu diễn trong {} và sẽ bị thay thế",
        "Bazı karakterler {} içinde gösterilemez ve değiştirilecek",
        "Некоторые символы невозможно представить в {}, они будут заменены",
        "Niektóre znaki nie mogą być przedstawione w {} i zostaną zastąpione";
    hex_parse_failed: "HEX 解析失败: {}", "HEX parse failed: {}",
        "HEX 解析失敗: {}",
        "HEX 解析に失敗: {}",
        "Error de análisis HEX: {}",
        "Analisi HEX non riuscita: {}",
        "Falha ao analisar HEX: {}",
        "Gagal mengurai HEX: {}",
        "Échec de l'analyse HEX : {}",
        "HEX-Analyse fehlgeschlagen: {}",
        "HEX 구문 분석 실패: {}",
        "Phân tích HEX thất bại: {}",
        "HEX ayrıştırma başarısız: {}",
        "Ошибка разбора HEX: {}",
        "Błąd analizy HEX: {}";
    history_browsing: "历史浏览中：↑ 更旧 / ↓ 更新", "Browsing history: ↑ older / ↓ newer",
        "歷史瀏覽中：↑ 更舊 / ↓ 更新",
        "履歴を参照中: ↑ 古い / ↓ 新しい",
        "Navegando por el historial: ↑ más antiguo / ↓ más reciente",
        "Sfogliando la cronologia: ↑ più vecchio / ↓ più recente",
        "Navegando no histórico: ↑ mais antigo / ↓ mais recente",
        "Menelusuri riwayat: ↑ lebih lama / ↓ lebih baru",
        "Navigation dans l'historique : ↑ plus ancien / ↓ plus récent",
        "Verlauf durchblättern: ↑ älter / ↓ neuer",
        "기록 탐색 중: ↑ 이전 / ↓ 최신",
        "Đang duyệt lịch sử: ↑ cũ hơn / ↓ mới hơn",
        "Geçmişe göz atılıyor: ↑ daha eski / ↓ daha yeni",
        "Просмотр истории: ↑ старее / ↓ новее",
        "Przeglądanie historii: ↑ starsze / ↓ nowsze";
    // 发送框右下角把手的悬停提示（2026-09-26 新增的可拖高把手）
    send_resize_hint: "上下拖动可调整输入框高度（双击恢复默认）",
        "Drag up/down to resize the input box (double-click to reset)",
        "上下拖曳可調整輸入框高度（雙擊恢復預設）",
        "上下にドラッグで入力欄の高さを変更（ダブルクリックで既定値に戻す）",
        "Arrastra arriba/abajo para cambiar la altura (doble clic para restablecer)",
        "Trascina su/giù per ridimensionare la casella (doppio clic per ripristinare)",
        "Arraste para cima/baixo para redimensionar a caixa (duplo clique para restaurar)",
        "Geser atas/bawah untuk mengubah tinggi kotak (klik dua kali untuk mengatur ulang)",
        "Glissez vers le haut/bas pour redimensionner la zone (double-clic pour réinitialiser)",
        "Zum Ändern der Höhe nach oben/unten ziehen (Doppelklick zum Zurücksetzen)",
        "위/아래로 드래그하여 입력창 높이 조절 (두 번 클릭하면 기본값으로 복원)",
        "Kéo lên/xuống để thay đổi chiều cao ô nhập (nhấp đúp để khôi phục mặc định)",
        "Yüksekliği değiştirmek için yukarı/aşağı sürükleyin (sıfırlamak için çift tıklayın)",
        "Перетащите вверх/вниз, чтобы изменить высоту поля (двойной клик — сброс)",
        "Przeciągnij w górę/w dół, aby zmienić wysokość pola (podwójne kliknięcie przywraca domyślną)";
    clear_send_button: "清空发送", "Clear input",
        "清空傳送",
        "入力を消去",
        "Borrar entrada",
        "Cancella input",
        "Limpar entrada",
        "Bersihkan masukan",
        "Effacer l'entrée",
        "Eingabe löschen",
        "입력 지우기",
        "Xóa nội dung",
        "Girdiyi temizle",
        "Очистить ввод",
        "Wyczyść dane";
    repeat_needs_connection:
        "未连接：连上之后才会开始定时发送",
        "Not connected — repeat send only starts once connected",
        "未連線：連上之後才會開始定時傳送",
        "未接続: 接続後に定期送信が始まります",
        "Sin conexión: el envío periódico solo empieza al conectarse",
        "Non connesso: l'invio periodico inizia solo dopo la connessione",
        "Não conectado: o envio periódico só começa após conectar",
        "Belum terhubung — pengiriman berkala baru dimulai setelah tersambung",
        "Non connecté : l'envoi périodique ne démarre qu'après connexion",
        "Nicht verbunden — wiederholtes Senden startet erst nach dem Verbinden",
        "연결되지 않음 — 연결 후에야 반복 전송이 시작됩니다",
        "Chưa kết nối — gửi định kỳ chỉ bắt đầu sau khi kết nối",
        "Bağlı değil — yineli gönderim ancak bağlandıktan sonra başlar",
        "Нет подключения — повторная отправка начнётся только после подключения",
        "Brak połączenia — wysyłanie cykliczne ruszy dopiero po połączeniu";
    escape: "转义", "Escape",
        "跳脫",
        "エスケープ",
        "Escape",
        "Escape",
        "Escape",
        "Escape",
        "Échappement",
        "Escape",
        "이스케이프",
        "Thoát ký tự",
        "Kaçış",
        "Экранирование",
        "Znaki ucieczki";

    // —— app.rs：常用数据（N2）——
    presets_header: "常用数据", "Presets",
        "常用資料",
        "定型データ",
        "Preajustes",
        "Preimpostazioni",
        "Predefinições",
        "Prasetel",
        "Préréglages",
        "Vorlagen",
        "즐겨찾기",
        "Mẫu sẵn dùng",
        "Hazır veriler",
        "Шаблоны",
        "Szablony";
    save_preset: "存为常用", "Save as preset",
        "存為常用",
        "定型として保存",
        "Guardar como preajuste",
        "Salva come preimpostazione",
        "Salvar como predefinição",
        "Simpan sebagai prasetel",
        "Enregistrer comme préréglage",
        "Als Vorlage speichern",
        "즐겨찾기에 저장",
        "Lưu thành mẫu",
        "Şablon olarak kaydet",
        "Сохранить как шаблон",
        "Zapisz jako szablon";
    preset_empty:
        "还没有常用数据：把输入框里的内容存进来，之后点一下就能重发",
        "No presets yet — save the input box contents, then send them with one click",
        "還沒有常用資料：把輸入框裡的內容存進來，之後點一下就能重送",
        "定型データはまだありません: 入力欄の内容を保存すると、あとでワンクリックで再送できます",
        "Aún no hay preajustes: guarda el contenido del cuadro de entrada y envíalo con un clic",
        "Nessuna preimpostazione: salva il contenuto della casella e invialo con un clic",
        "Ainda não há predefinições: salve o conteúdo da caixa de entrada e envie com um clique",
        "Belum ada prasetel — simpan isi kotak masukan, lalu kirim dengan sekali klik",
        "Aucun préréglage : enregistrez le contenu du champ de saisie, puis envoyez-le en un clic",
        "Noch keine Vorlagen — speichern Sie den Inhalt des Eingabefelds und senden Sie ihn per Klick",
        "즐겨찾기가 없습니다 — 입력 상자의 내용을 저장하면 클릭 한 번으로 전송할 수 있습니다",
        "Chưa có mẫu nào — lưu nội dung ô nhập rồi gửi bằng một cú nhấp",
        "Henüz hazır veri yok — giriş kutusundaki içeriği kaydedin, sonra tek tıkla gönderin",
        "Шаблонов пока нет — сохраните содержимое поля ввода и отправляйте его одним нажатием",
        "Brak szablonów — zapisz zawartość pola wprowadzania i wysyłaj ją jednym kliknięciem";
    preset_hint: "点一下发送；右键删除", "Click to send; right-click to delete",
        "點一下傳送；右鍵刪除",
        "クリックで送信、右クリックで削除",
        "Clic para enviar; clic derecho para borrar",
        "Clic per inviare; clic destro per eliminare",
        "Clique para enviar; clique com o botão direito para excluir",
        "Klik untuk mengirim; klik kanan untuk menghapus",
        "Cliquez pour envoyer ; clic droit pour supprimer",
        "Klicken zum Senden; Rechtsklick zum Löschen",
        "클릭하면 전송, 오른쪽 클릭하면 삭제",
        "Nhấp để gửi; nhấp chuột phải để xóa",
        "Göndermek için tıklayın; silmek için sağ tıklayın",
        "Нажмите, чтобы отправить; правый щелчок — удалить",
        "Kliknij, aby wysłać; kliknij prawym przyciskiem, aby usunąć";
    preset_full: "上限 {} 条", "Up to {} presets",
        "上限 {} 筆",
        "上限 {} 件",
        "Hasta {} preajustes",
        "Fino a {} preimpostazioni",
        "Até {} predefinições",
        "Maksimal {} prasetel",
        "Jusqu'à {} préréglages",
        "Bis zu {} Vorlagen",
        "최대 {} 개",
        "Tối đa {} mẫu",
        "En fazla {} şablon",
        "До {} шаблонов",
        "Maksymalnie {} szablonów";
    preset_duplicate: "已在列表中", "Already saved",
        "已在清單中",
        "すでに登録済み",
        "Ya guardado",
        "Già salvato",
        "Já salvo",
        "Sudah disimpan",
        "Déjà enregistré",
        "Bereits gespeichert",
        "이미 저장됨",
        "Đã lưu",
        "Zaten kaydedildi",
        "Уже сохранено",
        "Już zapisano";
    preset_input_empty: "输入框为空", "Input is empty",
        "輸入框為空",
        "入力欄が空です",
        "La entrada está vacía",
        "L'input è vuoto",
        "A entrada está vazia",
        "Masukan kosong",
        "L'entrée est vide",
        "Eingabe ist leer",
        "입력이 비어 있습니다",
        "Ô nhập trống",
        "Giriş kutusu boş",
        "Поле ввода пустое",
        "Pole wprowadzania jest puste";
    preset_count_text: "{} / {} 条", "{} / {} saved",
        "{} / {} 筆",
        "{} / {} 件",
        "{} / {} guardados",
        "{} / {} salvati",
        "{} / {} salvos",
        "{} / {} tersimpan",
        "{} / {} enregistrés",
        "{} / {} gespeichert",
        "{} / {} 저장됨",
        "{} / {} đã lưu",
        "{} / {} kayıtlı",
        "{} / {} сохранено",
        "{} / {} zapisanych";
    clear_presets: "清空常用数据", "Clear presets",
        "清空常用資料",
        "定型データを消去",
        "Borrar preajustes",
        "Cancella preimpostazioni",
        "Limpar predefinições",
        "Bersihkan prasetel",
        "Effacer les préréglages",
        "Vorlagen löschen",
        "즐겨찾기 지우기",
        "Xóa mẫu sẵn dùng",
        "Şablonları temizle",
        "Очистить шаблоны",
        "Wyczyść szablony";
    preset_needs_connection: "未连接：连上之后才能发送", "Not connected — cannot send yet",
        "未連線：連上之後才能傳送",
        "未接続: 接続後に送信できます",
        "Sin conexión: aún no se puede enviar",
        "Non connesso: impossibile inviare",
        "Não conectado: ainda não é possível enviar",
        "Belum terhubung — belum bisa mengirim",
        "Non connecté : envoi impossible pour l'instant",
        "Nicht verbunden — Senden noch nicht möglich",
        "연결되지 않음 — 아직 전송할 수 없습니다",
        "Chưa kết nối — chưa thể gửi",
        "Bağlı değil — henüz gönderilemez",
        "Нет подключения — отправка пока невозможна",
        "Brak połączenia — nie można jeszcze wysłać";

    // —— app.rs / log_file.rs：原生文件对话框 ——
    export_dialog_title: "导出接收数据.txt", "Export received data.txt",
        "匯出接收資料.txt",
        "受信データを書き出す.txt",
        "Exportar datos recibidos.txt",
        "Esporta dati ricevuti.txt",
        "Exportar dados recebidos.txt",
        "Ekspor data diterima.txt",
        "Exporter les données reçues.txt",
        "Empfangene Daten exportieren.txt",
        "수신 데이터 내보내기.txt",
        "Xuất dữ liệu nhận.txt",
        "Alınan verileri dışa aktar.txt",
        "Экспорт принятых данных.txt",
        "Eksport odebranych danych.txt";
    export_text_filter: "文本文件", "Text files",
        "文字檔",
        "テキストファイル",
        "Archivos de texto",
        "File di testo",
        "Arquivos de texto",
        "Berkas teks",
        "Fichiers texte",
        "Textdateien",
        "텍스트 파일",
        "Tệp văn bản",
        "Metin dosyaları",
        "Текстовые файлы",
        "Pliki tekstowe";
    pick_log_dir_title: "选择日志输出目录", "Choose log output directory",
        "選擇日誌輸出目錄",
        "ログ出力先フォルダを選択",
        "Elegir carpeta de registro",
        "Scegli la cartella dei log",
        "Escolher pasta de log",
        "Pilih direktori log",
        "Choisir le dossier de journalisation",
        "Log-Ausgabeordner wählen",
        "로그 출력 디렉터리 선택",
        "Chọn thư mục ghi log",
        "Günlük çıktı klasörünü seç",
        "Выбор папки журнала",
        "Wybierz katalog dziennika";

    // —— info.rs：关于窗口 ——
    info_window_title: "信息", "Info",
        "資訊",
        "情報",
        "Información",
        "Informazioni",
        "Informações",
        "Info",
        "Informations",
        "Info",
        "정보",
        "Thông tin",
        "Bilgi",
        "Информация",
        "Informacje";
    version_label: "版本 {}", "Version {}",
        "版本 {}",
        "バージョン {}",
        "Versión {}",
        "Versione {}",
        "Versão {}",
        "Versi {}",
        "Version {}",
        "Version {}",
        "버전 {}",
        "Phiên bản {}",
        "Sürüm {}",
        "Версия {}",
        "Wersja {}";
    app_description:
        "基于 eframe/egui 的串口调试上位机",
        "Serial port terminal built on eframe/egui",
        "基於 eframe/egui 的串口除錯上位機",
        "eframe/egui ベースのシリアルポート端末",
        "Terminal de puerto serie basado en eframe/egui",
        "Terminale seriale basato su eframe/egui",
        "Terminal de porta serial baseado em eframe/egui",
        "Terminal port serial berbasis eframe/egui",
        "Terminal de port série basé sur eframe/egui",
        "Serielles Terminal auf Basis von eframe/egui",
        "eframe/egui 기반 시리얼 포트 터미널",
        "Terminal cổng nối tiếp dựa trên eframe/egui",
        "eframe/egui tabanlı seri port terminali",
        "Терминал последовательного порта на eframe/egui",
        "Terminal portu szeregowego oparty na eframe/egui";
    serial_backend: "串口后端: serialport", "Serial backend: serialport",
        "串口後端: serialport",
        "シリアルバックエンド: serialport",
        "Backend serie: serialport",
        "Backend seriale: serialport",
        "Backend serial: serialport",
        "Backend serial: serialport",
        "Backend série : serialport",
        "Serielles Backend: serialport",
        "시리얼 백엔드: serialport",
        "Backend nối tiếp: serialport",
        "Seri port arka ucu: serialport",
        "Серверная часть порта: serialport",
        "Backend portu szeregowego: serialport";
    ui_framework: "界面框架: eframe / egui", "UI framework: eframe / egui",
        "介面框架: eframe / egui",
        "UI フレームワーク: eframe / egui",
        "Marco de UI: eframe / egui",
        "Framework UI: eframe / egui",
        "Framework de UI: eframe / egui",
        "Kerangka UI: eframe / egui",
        "Framework d'interface : eframe / egui",
        "UI-Framework: eframe / egui",
        "UI 프레임워크: eframe / egui",
        "Khung giao diện: eframe / egui",
        "Arayüz çatısı: eframe / egui",
        "Фреймворк интерфейса: eframe / egui",
        "Framework interfejsu: eframe / egui";
    settings_dir: "设置存档: {}", "Settings dir: {}",
        "設定存檔: {}",
        "設定の保存先: {}",
        "Carpeta de ajustes: {}",
        "Cartella impostazioni: {}",
        "Pasta de configurações: {}",
        "Direktori pengaturan: {}",
        "Dossier des paramètres : {}",
        "Einstellungsordner: {}",
        "설정 저장 위치: {}",
        "Thư mục cài đặt: {}",
        "Ayarlar klasörü: {}",
        "Каталог настроек: {}",
        "Katalog ustawień: {}";
    settings_dir_unavailable:
        "设置存档: 当前平台不适用",
        "Settings dir: not applicable on this platform",
        "設定存檔: 目前平台不適用",
        "設定の保存先: このプラットフォームでは未対応",
        "Carpeta de ajustes: no aplicable en esta plataforma",
        "Cartella impostazioni: non applicabile su questa piattaforma",
        "Pasta de configurações: não aplicável nesta plataforma",
        "Direktori pengaturan: tidak berlaku di platform ini",
        "Dossier des paramètres : non applicable sur cette plateforme",
        "Einstellungsordner: auf dieser Plattform nicht verfügbar",
        "설정 저장 위치: 이 플랫폼에서는 해당 없음",
        "Thư mục cài đặt: không áp dụng trên nền tảng này",
        "Ayarlar klasörü: bu platformda geçerli değil",
        "Каталог настроек: не применимо на этой платформе",
        "Katalog ustawień: nie dotyczy tej platformy";
    shortcuts: "快捷键", "Shortcuts",
        "快速鍵",
        "ショートカット",
        "Atajos",
        "Scorciatoie",
        "Atalhos",
        "Pintasan",
        "Raccourcis",
        "Tastenkürzel",
        "단축키",
        "Phím tắt",
        "Kısayollar",
        "Горячие клавиши",
        "Skróty klawiszowe";
    shortcut_send: "Enter 发送 / Shift+Enter 换行", "Enter to send / Shift+Enter for a newline",
        "Enter 傳送 / Shift+Enter 換行",
        "Enter で送信 / Shift+Enter で改行",
        "Enter para enviar / Shift+Enter para nueva línea",
        "Invio per inviare / Maiusc+Invio per nuova riga",
        "Enter para enviar / Shift+Enter para nova linha",
        "Enter untuk mengirim / Shift+Enter untuk baris baru",
        "Entrée pour envoyer / Maj+Entrée pour un saut de ligne",
        "Enter zum Senden / Umschalt+Enter für Zeilenumbruch",
        "Enter 전송 / Shift+Enter 줄바꿈",
        "Enter để gửi / Shift+Enter để xuống dòng",
        "Göndermek için Enter / yeni satır için Shift+Enter",
        "Enter — отправить / Shift+Enter — новая строка",
        "Enter — wyślij / Shift+Enter — nowy wiersz";
    shortcut_copy: "接收区右键：复制全部日志", "Receive area, right-click: copy the whole log",
        "接收區右鍵：複製全部日誌",
        "受信エリア右クリック: ログをすべてコピー",
        "Área de recepción, clic derecho: copiar todo el registro",
        "Area di ricezione, clic destro: copia tutto il log",
        "Área de recepção, clique direito: copiar todo o log",
        "Area penerimaan, klik kanan: salin seluruh log",
        "Zone de réception, clic droit : copier tout le journal",
        "Empfangsbereich, Rechtsklick: gesamtes Protokoll kopieren",
        "수신 영역 오른쪽 클릭: 전체 로그 복사",
        "Vùng nhận, nhấp chuột phải: sao chép toàn bộ log",
        "Alım alanı, sağ tıklama: tüm günlüğü kopyala",
        "Область приёма, правый щелчок: копировать весь журнал",
        "Obszar odbioru, kliknięcie prawym przyciskiem: kopiuj cały dziennik";

    // —— sidebar.rs：只读信息回显 ——
    nav_sidebar: "侧边栏", "Sidebar",
        "側邊欄",
        "サイドバー",
        "Barra lateral",
        "Barra laterale",
        "Barra lateral",
        "Bilah samping",
        "Barre latérale",
        "Seitenleiste",
        "사이드바",
        "Thanh bên",
        "Kenar çubuğu",
        "Боковая панель",
        "Pasek boczny";
    collapse: "收起", "Collapse",
        "收合",
        "折りたたむ",
        "Contraer",
        "Comprimi",
        "Recolher",
        "Ciutkan",
        "Réduire",
        "Einklappen",
        "접기",
        "Thu gọn",
        "Daralt",
        "Свернуть",
        "Zwiń";
    serial_info: "串口信息", "Port info",
        "串口資訊",
        "ポート情報",
        "Info del puerto",
        "Info porta",
        "Info da porta",
        "Info port",
        "Infos du port",
        "Port-Info",
        "포트 정보",
        "Thông tin cổng",
        "Port bilgisi",
        "Информация о порте",
        "Informacje o porcie";
    port_label: "端口: {}", "Port: {}",
        "連接埠: {}",
        "ポート: {}",
        "Puerto: {}",
        "Porta: {}",
        "Porta: {}",
        "Port: {}",
        "Port : {}",
        "Port: {}",
        "포트: {}",
        "Cổng: {}",
        "Port: {}",
        "Порт: {}",
        "Port: {}";
    not_selected: "未选择", "not selected",
        "未選擇",
        "未選択",
        "no seleccionado",
        "non selezionato",
        "não selecionado",
        "belum dipilih",
        "non sélectionné",
        "nicht ausgewählt",
        "선택 안 됨",
        "chưa chọn",
        "seçilmedi",
        "не выбрано",
        "nie wybrano";
    baud_label: "波特率: {}", "Baud rate: {}",
        "波特率: {}",
        "ボーレート: {}",
        "Velocidad: {}",
        "Baud rate: {}",
        "Taxa de transmissão: {}",
        "Baud rate: {}",
        "Débit : {}",
        "Baudrate: {}",
        "보드레이트: {}",
        "Tốc độ baud: {}",
        "Baud hızı: {}",
        "Скорость: {}",
        "Prędkość: {}";
    status_connected: "状态: 已连接", "Status: connected",
        "狀態: 已連線",
        "状態: 接続済み",
        "Estado: conectado",
        "Stato: connesso",
        "Status: conectado",
        "Status: terhubung",
        "État : connecté",
        "Status: verbunden",
        "상태: 연결됨",
        "Trạng thái: đã kết nối",
        "Durum: bağlı",
        "Состояние: подключено",
        "Stan: połączono";
    status_disconnected: "状态: 未连接", "Status: disconnected",
        "狀態: 未連線",
        "状態: 未接続",
        "Estado: desconectado",
        "Stato: disconnesso",
        "Status: desconectado",
        "Status: terputus",
        "État : déconnecté",
        "Status: getrennt",
        "상태: 연결 끊김",
        "Trạng thái: chưa kết nối",
        "Durum: bağlı değil",
        "Состояние: не подключено",
        "Stan: rozłączono";
    rx_bytes_label: "接收字节数: {}", "Received bytes: {}",
        "接收位元組數: {}",
        "受信バイト数: {}",
        "Bytes recibidos: {}",
        "Byte ricevuti: {}",
        "Bytes recebidos: {}",
        "Bita diterima: {}",
        "Octets reçus : {}",
        "Empfangene Bytes: {}",
        "수신 바이트: {}",
        "Byte đã nhận: {}",
        "Alınan bayt: {}",
        "Принято байт: {}",
        "Odebrane bajty: {}";
    tx_bytes_label: "发送字节数: {}", "Sent bytes: {}",
        "傳送位元組數: {}",
        "送信バイト数: {}",
        "Bytes enviados: {}",
        "Byte inviati: {}",
        "Bytes enviados: {}",
        "Bita dikirim: {}",
        "Octets envoyés : {}",
        "Gesendete Bytes: {}",
        "송신 바이트: {}",
        "Byte đã gửi: {}",
        "Gönderilen bayt: {}",
        "Отправлено байт: {}",
        "Wysłane bajty: {}";
    tx_bytes_hover:
        "自本次连接以来累计送出，断开后归零；与上面「接收字节数」口径不同——那个是当前缓冲区里的字节数，会被上限裁掉",
        "Counted since this connection and reset on disconnect; note the different meaning from \"Received bytes\" above, which is the size of the current buffer and is trimmed at the cap",
        "自本次連線以來累計送出，斷開後歸零；與上面「接收位元組數」口徑不同——那個是目前緩衝區裡的位元組數，會被上限裁掉",
        "この接続以降の累計送信量で、切断すると 0 に戻ります。上の「受信バイト数」とは意味が異なります——あちらは現在のバッファ内のバイト数で、上限で切り捨てられます",
        "Contado desde esta conexión y restablecido al desconectar; no confundir con \"Bytes recibidos\" arriba, que es el tamaño del búfer actual y se recorta al alcanzar el límite",
        "Contati da questa connessione e azzerati alla disconnessione; da non confondere con \"Byte ricevuti\" sopra, che è la dimensione del buffer attuale e viene ridotta al limite",
        "Contados desde esta conexão e zerados ao desconectar; não confundir com \"Bytes recebidos\" acima, que é o tamanho do buffer atual e é cortado no limite",
        "Dihitung sejak koneksi ini dan direset saat terputus; jangan disamakan dengan \"Bita diterima\" di atas, yaitu ukuran buffer saat ini yang dipangkas di batas",
        "Comptés depuis cette connexion et remis à zéro à la déconnexion ; à ne pas confondre avec « Octets reçus » ci-dessus, qui est la taille du tampon actuel, tronquée à la limite",
        "Seit dieser Verbindung gezählt und beim Trennen zurückgesetzt; nicht mit \"Empfangene Bytes\" oben verwechseln — das ist die Größe des aktuellen Puffers und wird am Limit gekürzt",
        "이 연결 이후 누적 송신량이며 연결이 끊기면 0으로 초기화됩니다. 위의 \"수신 바이트\"와는 의미가 다릅니다——그쪽은 현재 버퍼 크기로 상한에서 잘립니다",
        "Được tính từ lần kết nối này và đặt lại khi ngắt kết nối; khác với \"Byte đã nhận\" ở trên — đó là kích thước bộ đệm hiện tại và bị cắt ở giới hạn",
        "Bu bağlantıdan bu yana sayılır ve bağlantı kesilince sıfırlanır; yukarıdaki «Alınan bayt» ile aynı anlamda değildir — o, geçerli arabelleğin boyutudur ve üst sınıra gelince kırpılır",
        "Считается с момента этого подключения и сбрасывается при отключении; это не то же самое, что «Принято байт» выше — там размер текущего буфера, который обрезается по достижении предела",
        "Liczone od tego połączenia i zerowane przy rozłączeniu; to nie to samo co „Odebrane bajty” powyżej — tam jest rozmiar bieżącego bufora, przycinany po osiągnięciu limitu";
    rx_bytes_hover:
        "当前接收缓冲区里的字节数（不是本次连接累计收到的总量）；清空接收区时归零，达到上限会被裁掉",
        "The number of bytes currently held in the receive buffer (not a running total for this connection); reset when the receive area is cleared and trimmed at the cap",
        "目前接收緩衝區裡的位元組數（不是本次連線累計收到的總量）；清空接收區時歸零，達到上限會被裁掉",
        "現在の受信バッファ内のバイト数です（この接続での累計受信量ではありません）。受信エリアをクリアすると 0 に戻り、上限に達すると切り捨てられます",
        "Bytes que hay ahora mismo en el búfer de recepción (no es el total acumulado de esta conexión); se restablece al limpiar el área de recepción y se recorta al alcanzar el límite",
        "Byte attualmente presenti nel buffer di ricezione (non il totale accumulato di questa connessione); si azzera svuotando l'area di ricezione e viene ridotto al limite",
        "Bytes atualmente no buffer de receção (não é o total acumulado desta conexão); é zerado ao limpar a área de receção e cortado no limite",
        "Jumlah bita yang saat ini ada di buffer penerimaan (bukan total akumulatif koneksi ini); direset saat area penerimaan dibersihkan dan dipangkas di batas",
        "Octets actuellement dans le tampon de réception (et non le total cumulé de cette connexion) ; remis à zéro quand la zone de réception est vidée et tronqué à la limite",
        "Aktuell im Empfangspuffer liegende Bytes (nicht die Gesamtsumme dieser Verbindung); wird beim Leeren des Empfangsbereichs zurückgesetzt und am Limit gekürzt",
        "현재 수신 버퍼에 있는 바이트 수입니다(이 연결의 누적 수신량이 아님). 수신 영역을 지우면 0으로 돌아가고 상한에서 잘립니다",
        "Số byte hiện có trong bộ đệm nhận (không phải tổng tích lũy của kết nối này); đặt lại khi xóa vùng nhận và bị cắt ở giới hạn",
        "Alım arabelleğinde şu an bulunan bayt sayısı (bu bağlantının toplam alımı değil); alma alanı temizlendiğinde sıfırlanır ve üst sınırda kırpılır",
        "Число байтов, находящихся сейчас в буфере приёма (не суммарный объём за это подключение); сбрасывается при очистке области приёма и обрезается по достижении предела",
        "Liczba bajtów aktualnie znajdujących się w buforze odbioru (nie łączna suma tego połączenia); zerowana przy czyszczeniu obszaru odbioru i przycinana po osiągnięciu limitu";
    pending_bytes_label: "待发: {} 字节", "Pending: {} B",
        "待發: {} 位元組",
        "送信待ち: {} バイト",
        "Pendiente: {} bytes",
        "In attesa: {} byte",
        "Pendente: {} bytes",
        "Tertunda: {} byte",
        "En attente : {} octets",
        "Ausstehend: {} Bytes",
        "대기: {} 바이트",
        "Đang chờ: {} byte",
        "Bekleyen: {} bayt",
        "Ожидает: {} байт",
        "Oczekuje: {} bajtów";
    pending_bytes_hover:
        "已排进队列、还没写出去的字节数。这个数长时间不下降，说明设备或对端没有在接收——「发送看起来成功、对端却没收到」时，这是唯一能看到的线索",
        "Bytes queued but not yet written out. If this number stops falling, the device or peer is not accepting data — the only visible clue when a send looks successful but nothing arrives",
        "已排進佇列、還沒寫出去的位元組數。這個數長時間不下降，表示裝置或對端沒有在接收——「傳送看起來成功、對端卻沒收到」時，這是唯一能看到的線索",
        "キューに入ったがまだ送信されていないバイト数です。この数が長時間減らない場合は、デバイスまたは相手側がデータを受け取っていません——送信は成功したように見えるのに対向側へ届いていないとき、唯一の手がかりになります",
        "Bytes en cola que aún no se han escrito. Si este número no baja durante mucho tiempo, el dispositivo o el interlocutor no están aceptando datos: la única pista visible cuando el envío parece correcto pero no llega nada",
        "Byte in coda non ancora scritti. Se questo numero non scende per molto tempo, il dispositivo o l'interlocutore non stanno accettando dati: è l'unico indizio visibile quando l'invio sembra riuscito ma non arriva nulla",
        "Bytes em fila que ainda não foram gravados. Se este número não baixar durante muito tempo, o dispositivo ou o par não está aceitando dados — é a única pista visível quando o envio parece bem-sucedido mas nada chega",
        "Bita yang sudah masuk antrean tetapi belum ditulis. Jika angka ini lama tidak menurun, perangkat atau lawan bicara tidak menerima data — satu-satunya petunjuk saat pengiriman tampak berhasil tetapi tidak ada yang sampai",
        "Octets mis en file mais pas encore écrits. Si ce nombre ne baisse pas pendant longtemps, l'appareil ou le correspondant n'accepte plus de données — c'est le seul indice visible quand l'envoi semble réussir mais que rien n'arrive",
        "Bytes, die in der Warteschlange stehen, aber noch nicht geschrieben wurden. Sinkt diese Zahl längere Zeit nicht, nimmt das Gerät oder die Gegenstelle keine Daten mehr an — der einzige sichtbare Hinweis, wenn das Senden erfolgreich aussieht, aber nichts ankommt",
        "대기열에 들어갔지만 아직 기록되지 않은 바이트 수입니다. 이 숫자가 오랫동안 줄지 않으면 장치나 상대방이 데이터를 받지 않고 있다는 뜻입니다——전송은 성공한 것처럼 보이는데 상대방이 받지 못할 때 볼 수 있는 유일한 단서입니다",
        "Số byte đã vào hàng đợi nhưng chưa được ghi ra. Nếu con số này lâu không giảm, thiết bị hoặc đầu kia không nhận dữ liệu — đây là manh mối duy nhất khi việc gửi có vẻ thành công mà không có gì đến",
        "Kuyruğa girmiş ama henüz yazılmamış bayt sayısı. Bu sayı uzun süre düşmüyorsa cihaz veya karşı taraf veri kabul etmiyor demektir — gönderim başarılı görünüp karşı tarafa hiçbir şey ulaşmadığında görülebilen tek ipucu budur",
        "Байты, поставленные в очередь, но ещё не записанные. Если это число долго не уменьшается, устройство или собеседник не принимает данные — единственная видимая подсказка, когда отправка выглядит успешной, а на другом конце ничего не приходит",
        "Bajty w kolejce, które nie zostały jeszcze zapisane. Jeśli ta liczba długo nie spada, urządzenie lub druga strona nie przyjmuje danych — to jedyna widoczna wskazówka, gdy wysyłka wygląda na udaną, a nic nie dociera";
    send_file: "发送文件", "Send file",
        "傳送檔案",
        "ファイル送信",
        "Enviar archivo",
        "Invia file",
        "Enviar ficheiro",
        "Kirim berkas",
        "Envoyer un fichier",
        "Datei senden",
        "파일 보내기",
        "Gửi tệp",
        "Dosya gönder",
        "Отправить файл",
        "Wyślij plik";
    send_file_cancel: "取消发送", "Cancel send",
        "取消傳送",
        "送信を中止",
        "Cancelar envío",
        "Annulla invio",
        "Cancelar envio",
        "Batalkan kirim",
        "Annuler l'envoi",
        "Senden abbrechen",
        "보내기 취소",
        "Hủy gửi",
        "Gönderimi iptal et",
        "Отменить отправку",
        "Anuluj wysyłanie";
    send_file_progress: "发送文件: {} / {} 字节", "Sending file: {} / {} bytes",
        "傳送檔案: {} / {} 位元組",
        "ファイル送信中: {} / {} バイト",
        "Enviando archivo: {} / {} bytes",
        "Invio file: {} / {} byte",
        "A enviar ficheiro: {} / {} bytes",
        "Mengirim berkas: {} / {} bita",
        "Envoi du fichier : {} / {} octets",
        "Datei wird gesendet: {} / {} Bytes",
        "파일 전송 중: {} / {} 바이트",
        "Đang gửi tệp: {} / {} byte",
        "Dosya gönderiliyor: {} / {} bayt",
        "Отправка файла: {} / {} байт",
        "Wysyłanie pliku: {} / {} bajtów";
    log_config_locked_hint:
        "记录中：文件名与输出目录的改动，会在下次开始记录时生效",
        "Recording: changes to the file name and output folder take effect the next time you start recording",
        "記錄中：檔名與輸出目錄的變更，會在下次開始記錄時生效",
        "記録中: ファイル名と出力先の変更は、次に記録を開始したときから有効になります",
        "Grabando: los cambios de nombre de archivo y carpeta de salida se aplican la próxima vez que inicies la grabación",
        "Registrazione in corso: le modifiche a nome file e cartella di output valgono dalla prossima registrazione",
        "A gravar: as alterações de nome de ficheiro e pasta de saída só se aplicam na próxima gravação",
        "Sedang merekam: perubahan nama berkas dan folder keluaran berlaku saat mulai merekam berikutnya",
        "Enregistrement en cours : les changements de nom de fichier et de dossier de sortie s'appliquent au prochain démarrage",
        "Aufzeichnung läuft: Änderungen an Dateiname und Ausgabeordner gelten beim nächsten Start",
        "기록 중: 파일 이름과 출력 폴더 변경은 다음 기록 시작부터 적용됩니다",
        "Đang ghi: thay đổi tên tệp và thư mục đầu ra có hiệu lực từ lần bắt đầu ghi tiếp theo",
        "Kayıt sürüyor: dosya adı ve çıktı klasörü değişiklikleri bir sonraki kayıt başlangıcında geçerli olur",
        "Идёт запись: изменения имени файла и папки вывода вступят в силу при следующем запуске записи",
        "Trwa zapis: zmiany nazwy pliku i folderu wyjściowego zadziałają przy następnym rozpoczęciu zapisu";
    display_mode_line: "显示模式: {}", "Display mode: {}",
        "顯示模式: {}",
        "表示モード: {}",
        "Modo de vista: {}",
        "Modalità vista: {}",
        "Modo de exibição: {}",
        "Mode tampilan: {}",
        "Mode d'affichage : {}",
        "Anzeigemodus: {}",
        "표시 모드: {}",
        "Chế độ hiển thị: {}",
        "Görüntüleme modu: {}",
        "Режим отображения: {}",
        "Tryb wyświetlania: {}";
    suffix_color: " + 颜色", " + color",
        " + 顏色",
        " + 色",
        " + color",
        " + colore",
        " + cor",
        " + warna",
        " + couleur",
        " + Farbe",
        " + 색상",
        " + màu",
        " + renk",
        " + цвет",
        " + kolor";
    suffix_byte_view: " + 字节查看", " + byte view",
        " + 位元組檢視",
        " + バイト表示",
        " + vista de bytes",
        " + vista byte",
        " + vista de bytes",
        " + tampilan bita",
        " + vue octets",
        " + Byte-Ansicht",
        " + 바이트 보기",
        " + xem byte",
        " + bayt görünümü",
        " + просмотр байтов",
        " + widok bajtów";
    timestamp_prefix_line: "时间戳前缀: {}", "Timestamp prefix: {}",
        "時間戳前綴: {}",
        "タイムスタンプ接頭辞: {}",
        "Prefijo de marca de tiempo: {}",
        "Prefisso timestamp: {}",
        "Prefixo de carimbo de tempo: {}",
        "Awalan stempel waktu: {}",
        "Préfixe d'horodatage : {}",
        "Zeitstempel-Präfix: {}",
        "타임스탬프 접두사: {}",
        "Tiền tố dấu thời gian: {}",
        "Zaman damgası öneki: {}",
        "Префикс метки времени: {}",
        "Prefiks znacznika czasu: {}";
    available_ports: "可用串口", "Available ports",
        "可用串口",
        "利用可能なポート",
        "Puertos disponibles",
        "Porte disponibili",
        "Portas disponíveis",
        "Port tersedia",
        "Ports disponibles",
        "Verfügbare Ports",
        "사용 가능한 포트",
        "Cổng khả dụng",
        "Kullanılabilir portlar",
        "Доступные порты",
        "Dostępne porty";
    none_placeholder: "（无）", "(none)",
        "（無）",
        "（なし）",
        "(ninguno)",
        "(nessuno)",
        "(nenhum)",
        "(tidak ada)",
        "(aucun)",
        "(keine)",
        "(없음)",
        "(không có)",
        "(yok)",
        "(нет)",
        "(brak)";
    // 侧边栏那一行状态词：串口已发起打开、但端口还没真正打开。
    //
    // 与「已连接」分开写是本条缺陷（`connected` 假阳性）的**呈现落点**：分开之前
    // 用户在这一小段里看到的是"已连接"，点发送才发现发不出去。
    status_opening: "状态: 正在打开", "Status: opening",
        "狀態: 正在開啟",
        "状態: 開いています",
        "Estado: abriendo",
        "Stato: apertura",
        "Status: abrindo",
        "Status: membuka",
        "État : ouverture",
        "Status: wird geöffnet",
        "상태: 여는 중",
        "Trạng thái: đang mở",
        "Durum: açılıyor",
        "Состояние: открывается",
        "Stan: otwieranie";
    // 连接按钮旁边那句"已经点了、正在等"的弱提示。
    //
    // 与「请先选择端口」是两回事，不能共用：那条说"点下去不会有反应"，这条说
    // "点了，正在等结果"。
    opening_hint: "正在打开…", "Opening…",
        "正在開啟…",
        "開いています…",
        "Abriendo…",
        "Apertura…",
        "Abrindo…",
        "Membuka…",
        "Ouverture…",
        "Wird geöffnet…",
        "여는 중…",
        "Đang mở…",
        "Açılıyor…",
        "Открывается…",
        "Otwieranie…";

    // —— status_bar.rs：底部状态栏 ——
    // 这里刻意是**短词**（"已连接" / "未连接"），与侧边栏那套 "状态: 已连接" 分开写：
    // 侧边栏是一行行读数（前缀"状态:"有助于扫读），状态栏是常驻指示，前缀是噪音。
    // 两者必须给出**同一结论**，所以词义按同一套四态来（含 UDP 的"已绑定/未绑定"，
    // 见 `ConnectionKind` 的文档：UDP 没有连接，说"已连接"会让用户以为对端在线）。
    statusbar_connected: "已连接", "Connected",
        "已連線",
        "接続済み",
        "Conectado",
        "Connesso",
        "Conectado",
        "Terhubung",
        "Connecté",
        "Verbunden",
        "연결됨",
        "Đã kết nối",
        "Bağlı",
        "Подключено",
        "Połączono";
    statusbar_disconnected: "未连接", "Not connected",
        "未連線",
        "未接続",
        "Desconectado",
        "Disconnesso",
        "Desconectado",
        "Terputus",
        "Déconnecté",
        "Getrennt",
        "연결 끊김",
        "Chưa kết nối",
        "Bağlı değil",
        "Не подключено",
        "Rozłączono";
    statusbar_bound: "已绑定", "Bound",
        "已綁定",
        "バインド済み",
        "Vinculado",
        "Associato",
        "Vinculado",
        "Terikat",
        "Associé",
        "Gebunden",
        "바인딩됨",
        "Đã liên kết",
        "Bağlandı",
        "Привязано",
        "Powiązano";
    statusbar_unbound: "未绑定", "Not bound",
        "未綁定",
        "未バインド",
        "Sin vincular",
        "Non associato",
        "Não vinculado",
        "Tidak terikat",
        "Non associé",
        "Nicht gebunden",
        "바인딩 안 됨",
        "Chưa liên kết",
        "Bağlı değil",
        "Не привязано",
        "Niepowiązano";
    // 状态栏的短词版「正在打开」——与上面 `status_opening` 的唯一区别是没有
    // "状态:"前缀（状态栏是常驻指示，前缀是噪音；这条口径与
    // `statusbar_connected` 对 `status_connected` 完全一致）。
    statusbar_opening: "正在打开", "Opening",
        "正在開啟",
        "開いています",
        "Abriendo",
        "Apertura",
        "Abrindo",
        "Membuka",
        "Ouverture",
        "Wird geöffnet",
        "여는 중",
        "Đang mở",
        "Açılıyor",
        "Открывается",
        "Otwieranie";
    // 圆点**不是一个字符**，是用 `Color32` 画的实心圆（见 `status_bar.rs`）。
    // 早期版本用 🟢/🔴 两个 emoji，但 egui 的文本管线只光栅化字体的**矢量轮廓**，
    // 拿不到 COLR/CPAL 彩色图层，结果是两个大小不一、颜色灰黑的圆——不是字体选错，
    // 是根本渲染不了彩色字形。整个状态栏已不依赖任何 emoji 字体。

    // —— sidebar.rs：连接参数控件 ——
    port_field: "端口", "Port",
        "連接埠",
        "ポート",
        "Puerto",
        "Porta",
        "Porta",
        "Port",
        "Port",
        "Port",
        "포트",
        "Cổng",
        "Port",
        "Порт",
        "Port";
    select_port: "请选择端口", "Select a port",
        "請選擇連接埠",
        "ポートを選択",
        "Seleccione un puerto",
        "Seleziona una porta",
        "Selecione uma porta",
        "Pilih port",
        "Sélectionnez un port",
        "Port auswählen",
        "포트 선택",
        "Chọn cổng",
        "Bir port seçin",
        "Выберите порт",
        "Wybierz port";
    refresh: "刷新", "Refresh",
        "重新整理",
        "更新",
        "Actualizar",
        "Aggiorna",
        "Atualizar",
        "Muat ulang",
        "Actualiser",
        "Aktualisieren",
        "새로고침",
        "Làm mới",
        "Yenile",
        "Обновить",
        "Odśwież";
    baud_field: "波特率", "Baud rate",
        "波特率",
        "ボーレート",
        "Velocidad",
        "Baud rate",
        "Taxa de transmissão",
        "Baud rate",
        "Débit",
        "Baudrate",
        "보드레이트",
        "Tốc độ baud",
        "Baud hızı",
        "Скорость",
        "Prędkość";
    baud_custom_entry: "自定义…", "Custom…",
        "自訂…",
        "カスタム…",
        "Personalizado…",
        "Personalizzato…",
        "Personalizado…",
        "Kustom…",
        "Personnalisé…",
        "Benutzerdefiniert…",
        "사용자 지정…",
        "Tùy chỉnh…",
        "Özel…",
        "Другое…",
        "Niestandardowa…";
    baud_custom_hint: "可输入 {} ~ {} 之间的任意值", "Any value from {} to {}",
        "可輸入 {} ~ {} 之間的任何值",
        "{} ~ {} の任意の値を入力できます",
        "Cualquier valor de {} a {}",
        "Qualsiasi valore da {} a {}",
        "Qualquer valor de {} a {}",
        "Nilai apa pun dari {} hingga {}",
        "N'importe quelle valeur de {} à {}",
        "Beliebiger Wert von {} bis {}",
        "{} ~ {} 사이의 임의 값",
        "Bất kỳ giá trị nào từ {} đến {}",
        "{} ile {} arasında herhangi bir değer",
        "Любое значение от {} до {}",
        "Dowolna wartość od {} do {}";
    line_state_header: "控制线", "Control lines",
        "控制線",
        "制御線",
        "Líneas de control",
        "Linee di controllo",
        "Linhas de controle",
        "Garis kendali",
        "Lignes de contrôle",
        "Steuerleitungen",
        "제어선",
        "Đường điều khiển",
        "Kontrol hatları",
        "Управляющие линии",
        "Linie sterujące";
    line_state_hint:
        "连接后可手动控制；打开端口本身不会去动这两根线",
        "Controllable while connected; opening a port never touches these lines",
        "連線後可手動控制；開啟連接埠本身不會去動這兩根線",
        "接続後に手動制御できます。ポートを開くだけではこの 2 本の線には触れません",
        "Controlable al conectar; abrir el puerto no toca estas líneas",
        "Controllabili una volta connessi; aprire la porta non tocca queste linee",
        "Controlável ao conectar; abrir a porta não mexe nestas linhas",
        "Dapat dikendalikan saat terhubung; membuka port tidak menyentuh kedua jalur ini",
        "Contrôlables une fois connecté ; l'ouverture du port ne touche pas ces lignes",
        "Nach dem Verbinden steuerbar; das Öffnen des Ports beeinflusst diese Leitungen nicht",
        "연결 후 수동 제어 가능하며, 포트를 열어도 이 두 선에는 손대지 않습니다",
        "Điều khiển thủ công được sau khi kết nối; mở cổng không đụng đến hai đường này",
        "Bağlıyken elle kontrol edilebilir; portu açmak bu iki hatta dokunmaz",
        "Управляются вручную при подключении; открытие порта этих линий не касается",
        "Sterowane ręcznie po połączeniu; otwarcie portu nie dotyka tych linii";
    line_state_hover:
        "勾选表示「我们请求的电平」，不是对引脚的测量——驱动只接受写、不回读，所以硬件把线拉到哪里，界面看不到",
        "The checkbox is the level we requested, not a measurement — the driver only accepts writes and cannot read the lines back, so the UI cannot show where the hardware actually pulled them",
        "勾選表示「我們請求的電平」，不是對引腳的量測——驅動只接受寫、不回讀，所以硬體把線拉到哪裡，介面看不到",
        "チェックは「こちらが要求したレベル」であり、ピンの実測値ではありません。ドライバは書き込みのみで読み戻せないため、ハードウェアが実際にどのレベルに引いたかは UI では分かりません",
        "La casilla es el nivel que solicitamos, no una medición — el controlador solo acepta escrituras y no puede leer las líneas, así que la interfaz no puede mostrar dónde las puso el hardware",
        "La casella è il livello che abbiamo richiesto, non una misura — il driver accetta solo scritture e non può rileggere le linee, quindi l'interfaccia non può mostrare dove il hardware le ha effettivamente portate",
        "A caixa é o nível que solicitamos, não uma medição — o driver só aceita escrita e não relê as linhas, então a interface não mostra onde o hardware as colocou",
        "Kotak centang adalah level yang kami minta, bukan pengukuran — driver hanya menerima tulis dan tidak bisa membaca balik, jadi antarmuka tidak bisa menampilkan ke mana perangkat keras menariknya",
        "La case indique le niveau que nous demandons, pas une mesure — le pilote n'accepte que l'écriture et ne relit pas les lignes, l'interface ne peut donc pas montrer où le matériel les a réellement placées",
        "Das Kästchen ist der angeforderte Pegel, keine Messung — der Treiber akzeptiert nur Schreiben und liest die Leitungen nicht zurück, daher kann die Oberfläche nicht anzeigen, wohin die Hardware sie tatsächlich gezogen hat",
        "체크박스는 \"우리가 요청한 레벨\"이며 핀 측정값이 아닙니다——드라이버는 쓰기만 받고 다시 읽지 않으므로, 하드웨어가 선을 어디로 끌었는지 UI에서는 알 수 없습니다",
        "Hộp kiểm là mức chúng ta yêu cầu, không phải kết quả đo — trình điều khiển chỉ nhận ghi mà không đọc lại, nên giao diện không thể hiển thị mức thực tế mà phần cứng đã đặt",
        "Onay kutusu istediğimiz seviyeyi gösterir, ölçüm değildir — sürücü yalnızca yazmayı kabul eder ve hatları geri okuyamaz, bu yüzden arayüz donanımın hatları gerçekte nereye çektiğini gösteremez",
        "Флажок — это запрошенный нами уровень, а не измерение: драйвер принимает только запись и не может прочитать линии обратно, поэтому интерфейс не покажет, куда оборудование их фактически подтянуло",
        "Pole wyboru to żądany przez nas poziom, a nie pomiar — sterownik przyjmuje tylko zapis i nie odczytuje linii z powrotem, więc interfejs nie pokaże, dokąd sprzęt faktycznie je ściągnął";
    open_port_button: "打开串口", "Open",
        "開啟連接埠",
        "ポートを開く",
        "Abrir",
        "Apri",
        "Abrir",
        "Buka",
        "Ouvrir",
        "Öffnen",
        "열기",
        "Mở",
        "Aç",
        "Открыть",
        "Otwórz";
    close_port_button: "关闭串口", "Close",
        "關閉連接埠",
        "ポートを閉じる",
        "Cerrar",
        "Chiudi",
        "Fechar",
        "Tutup",
        "Fermer",
        "Schließen",
        "닫기",
        "Đóng",
        "Kapat",
        "Закрыть",
        "Zamknij";
    data_bits: "数据位", "Data bits",
        "資料位元",
        "データビット",
        "Bits de datos",
        "Bit di dati",
        "Bits de dados",
        "Bit data",
        "Bits de données",
        "Datenbits",
        "데이터 비트",
        "Bit dữ liệu",
        "Veri biti",
        "Биты данных",
        "Bity danych";
    stop_bits: "停止位", "Stop bits",
        "停止位元",
        "ストップビット",
        "Bits de parada",
        "Bit di stop",
        "Bits de parada",
        "Bit henti",
        "Bits d'arrêt",
        "Stoppbits",
        "정지 비트",
        "Bit dừng",
        "Durdurma biti",
        "Стоп-биты",
        "Bity stopu";
    parity: "校验", "Parity",
        "同位檢查",
        "パリティ",
        "Paridad",
        "Parità",
        "Paridade",
        "Paritas",
        "Parité",
        "Parität",
        "패리티",
        "Parity",
        "Eşlik",
        "Чётность",
        "Parzystość";
    flow_control: "流控", "Flow control",
        "流量控制",
        "フロー制御",
        "Control de flujo",
        "Controllo di flusso",
        "Controle de fluxo",
        "Kendali alir",
        "Contrôle de flux",
        "Flusskontrolle",
        "흐름 제어",
        "Điều khiển luồng",
        "Akış denetimi",
        "Управление потоком",
        "Sterowanie przepływem";

    // —— sidebar.rs：连接类型与网络参数（F4）——
    connection_type: "连接类型", "Connection",
        "連線類型",
        "接続タイプ",
        "Conexión",
        "Connessione",
        "Conexão",
        "Koneksi",
        "Connexion",
        "Verbindung",
        "연결 유형",
        "Kết nối",
        "Bağlantı",
        "Подключение",
        "Połączenie";
    conn_serial: "串口", "Serial",
        "串口",
        "シリアル",
        "Serie",
        "Seriale",
        "Serial",
        "Serial",
        "Série",
        "Seriell",
        "시리얼",
        "Nối tiếp",
        "Seri",
        "Последовательный",
        "Szeregowy";
    conn_tcp: "TCP 客户端", "TCP client",
        "TCP 用戶端",
        "TCP クライアント",
        "Cliente TCP",
        "Client TCP",
        "Cliente TCP",
        "Klien TCP",
        "Client TCP",
        "TCP-Client",
        "TCP 클라이언트",
        "Máy khách TCP",
        "TCP istemcisi",
        "TCP-клиент",
        "Klient TCP";
    conn_udp: "UDP", "UDP",
        "UDP",
        "UDP",
        "UDP",
        "UDP",
        "UDP",
        "UDP",
        "UDP",
        "UDP",
        "UDP",
        "UDP",
        "UDP",
        "UDP",
        "UDP";
    connection_info: "连接信息", "Connection info",
        "連線資訊",
        "接続情報",
        "Info de conexión",
        "Info connessione",
        "Info da conexão",
        "Info koneksi",
        "Infos de connexion",
        "Verbindungsinfo",
        "연결 정보",
        "Thông tin kết nối",
        "Bağlantı bilgisi",
        "Информация о подключении",
        "Informacje o połączeniu";
    target_label: "目标: {}", "Target: {}",
        "目標: {}",
        "ターゲット: {}",
        "Destino: {}",
        "Destinazione: {}",
        "Destino: {}",
        "Target: {}",
        "Cible : {}",
        "Ziel: {}",
        "대상: {}",
        "Đích: {}",
        "Hedef: {}",
        "Цель: {}",
        "Cel: {}";
    remote_host_field: "远端地址", "Remote host",
        "遠端位址",
        "リモートホスト",
        "Host remoto",
        "Host remoto",
        "Host remoto",
        "Host jarak jauh",
        "Hôte distant",
        "Remote-Host",
        "원격 호스트",
        "Máy chủ từ xa",
        "Uzak ana bilgisayar",
        "Удалённый узел",
        "Zdalny host";
    remote_host_hint: "IP 或域名", "IP or hostname",
        "IP 或網域名稱",
        "IP またはホスト名",
        "IP o nombre de host",
        "IP o nome host",
        "IP ou nome de host",
        "IP atau nama host",
        "IP ou nom d'hôte",
        "IP oder Hostname",
        "IP 또는 호스트 이름",
        "IP hoặc tên miền",
        "IP veya ana bilgisayar adı",
        "IP или имя узла",
        "IP lub nazwa hosta";
    remote_port_field: "远端端口", "Remote port",
        "遠端連接埠",
        "リモートポート",
        "Puerto remoto",
        "Porta remota",
        "Porta remota",
        "Port jarak jauh",
        "Port distant",
        "Remote-Port",
        "원격 포트",
        "Cổng từ xa",
        "Uzak port",
        "Удалённый порт",
        "Zdalny port";
    local_port_field: "本地端口", "Local port",
        "本機連接埠",
        "ローカルポート",
        "Puerto local",
        "Porta locale",
        "Porta local",
        "Port lokal",
        "Port local",
        "Lokaler Port",
        "로컬 포트",
        "Cổng cục bộ",
        "Yerel port",
        "Локальный порт",
        "Port lokalny";
    local_port_line: "本地端口: {}", "Local port: {}",
        "本機連接埠: {}",
        "ローカルポート: {}",
        "Puerto local: {}",
        "Porta locale: {}",
        "Porta local: {}",
        "Port lokal: {}",
        "Port local : {}",
        "Lokaler Port: {}",
        "로컬 포트: {}",
        "Cổng cục bộ: {}",
        "Yerel port: {}",
        "Локальный порт: {}",
        "Port lokalny: {}";
    local_port_hint: "0 = 由系统分配", "0 = assigned by the system",
        "0 = 由系統分配",
        "0 = システムが自動割り当て",
        "0 = asignado por el sistema",
        "0 = assegnata dal sistema",
        "0 = atribuída pelo sistema",
        "0 = ditentukan sistem",
        "0 = attribué par le système",
        "0 = vom System zugewiesen",
        "0 = 시스템이 자동 할당",
        "0 = hệ thống tự cấp",
        "0 = sistem atar",
        "0 = назначается системой",
        "0 = przydziela system";
    connect_button: "连接", "Connect",
        "連線",
        "接続",
        "Conectar",
        "Connetti",
        "Conectar",
        "Hubungkan",
        "Connecter",
        "Verbinden",
        "연결",
        "Kết nối",
        "Bağlan",
        "Подключить",
        "Połącz";
    disconnect_button: "断开", "Disconnect",
        "中斷",
        "切断",
        "Desconectar",
        "Disconnetti",
        "Desconectar",
        "Putuskan",
        "Déconnecter",
        "Trennen",
        "연결 끊기",
        "Ngắt kết nối",
        "Bağlantıyı kes",
        "Отключить",
        "Rozłącz";
    bind_button: "绑定", "Bind",
        "綁定",
        "バインド",
        "Vincular",
        "Associa",
        "Vincular",
        "Ikat",
        "Associer",
        "Binden",
        "바인딩",
        "Liên kết",
        "Bağla",
        "Привязать",
        "Powiąż";
    status_bound: "状态: 已绑定", "Status: bound",
        "狀態: 已綁定",
        "状態: バインド済み",
        "Estado: vinculado",
        "Stato: associato",
        "Status: vinculado",
        "Status: terikat",
        "État : associé",
        "Status: gebunden",
        "상태: 바인딩됨",
        "Trạng thái: đã liên kết",
        "Durum: bağlandı",
        "Состояние: привязано",
        "Stan: powiązano";
    status_unbound: "状态: 未绑定", "Status: not bound",
        "狀態: 未綁定",
        "状態: 未バインド",
        "Estado: sin vincular",
        "Stato: non associato",
        "Status: não vinculado",
        "Status: tidak terikat",
        "État : non associé",
        "Status: nicht gebunden",
        "상태: 바인딩 안 됨",
        "Trạng thái: chưa liên kết",
        "Durum: bağlı değil",
        "Состояние: не привязано",
        "Stan: niepowiązane";
    switch_kind_needs_disconnect:
        "连接中不能切换连接类型：请先断开，否则界面显示的参数与实际打开的通道会对不上",
        "Cannot switch the connection type while connected — disconnect first, or the shown parameters will not match the open channel",
        "連線中不能切換連線類型：請先中斷，否則介面顯示的參數與實際開啟的通道會對不上",
        "接続中は接続タイプを切り替えられません。先に切断してください。でないと表示中のパラメータと実際に開いているチャネルが食い違います",
        "No se puede cambiar el tipo de conexión mientras está conectado: desconecte primero, o los parámetros mostrados no coincidirán con el canal abierto",
        "Impossibile cambiare il tipo di connessione mentre si è connessi: disconnettersi prima, altrimenti i parametri mostrati non corrisponderanno al canale aperto",
        "Não é possível trocar o tipo de conexão conectado: desconecte primeiro, ou os parâmetros exibidos não corresponderão ao canal aberto",
        "Jenis koneksi tidak dapat diganti saat terhubung — putuskan dulu, jika tidak parameter yang ditampilkan tidak akan sesuai dengan kanal yang terbuka",
        "Impossible de changer le type de connexion en cours de connexion : déconnectez-vous d'abord, sinon les paramètres affichés ne correspondront pas au canal ouvert",
        "Der Verbindungstyp kann nicht während einer bestehenden Verbindung gewechselt werden — erst trennen, sonst passen die angezeigten Parameter nicht zum geöffneten Kanal",
        "연결 중에는 연결 유형을 바꿀 수 없습니다 — 먼저 연결을 끊으세요. 그렇지 않으면 표시된 파라미터와 실제 열린 채널이 어긋납니다",
        "Không thể đổi loại kết nối khi đang kết nối — hãy ngắt kết nối trước, nếu không thông số hiển thị sẽ không khớp với kênh đang mở",
        "Bağlıyken bağlantı türü değiştirilemez — önce bağlantıyı kesin, yoksa gösterilen parametreler açık kanalla uyuşmaz",
        "Нельзя сменить тип подключения во время соединения — сначала отключитесь, иначе показанные параметры не будут соответствовать открытому каналу",
        "Nie można zmienić typu połączenia podczas połączenia — najpierw rozłącz się, inaczej wyświetlane parametry nie będą zgodne z otwartym kanałem";
    need_port_to_connect: "请先选择端口", "Select a port first",
        "請先選擇連接埠",
        "先にポートを選択してください",
        "Seleccione primero un puerto",
        "Seleziona prima una porta",
        "Selecione primeiro uma porta",
        "Pilih port terlebih dahulu",
        "Sélectionnez d'abord un port",
        "Zuerst einen Port auswählen",
        "먼저 포트를 선택하세요",
        "Chọn cổng trước",
        "Önce bir port seçin",
        "Сначала выберите порт",
        "Najpierw wybierz port";
    need_target_to_connect: "请先填写目标地址与端口", "Enter a target host and port first",
        "請先填寫目標位址與連接埠",
        "先にターゲットのホストとポートを入力してください",
        "Introduzca primero un host y puerto de destino",
        "Inserisci prima host e porta di destinazione",
        "Informe primeiro um host e porta de destino",
        "Isi dulu host dan port target",
        "Saisissez d'abord un hôte et un port cibles",
        "Zuerst Zielhost und Port eingeben",
        "먼저 대상 호스트와 포트를 입력하세요",
        "Nhập máy chủ và cổng đích trước",
        "Önce hedef ana bilgisayarı ve portu girin",
        "Сначала укажите целевой узел и порт",
        "Najpierw podaj host i port docelowy";

    // —— sidebar.rs：日志操作 ——
    clear_log: "清空日志", "Clear log",
        "清空日誌",
        "ログを消去",
        "Borrar registro",
        "Cancella log",
        "Limpar log",
        "Bersihkan log",
        "Effacer le journal",
        "Protokoll löschen",
        "로그 지우기",
        "Xóa log",
        "Günlüğü temizle",
        "Очистить журнал",
        "Wyczyść dziennik";
    export_log: "导出日志", "Export log",
        "匯出日誌",
        "ログを書き出す",
        "Exportar registro",
        "Esporta log",
        "Exportar log",
        "Ekspor log",
        "Exporter le journal",
        "Protokoll exportieren",
        "로그 내보내기",
        "Xuất log",
        "Günlüğü dışa aktar",
        "Экспорт журнала",
        "Eksportuj dziennik";
    file_log: "文件日志", "File log",
        "檔案日誌",
        "ファイルログ",
        "Registro en archivo",
        "Log su file",
        "Log em arquivo",
        "Log berkas",
        "Journal fichier",
        "Dateiprotokoll",
        "파일 로그",
        "Log tệp",
        "Dosya günlüğü",
        "Журнал в файл",
        "Dziennik do pliku";
    stop_logging: "停止记录", "Stop logging",
        "停止記錄",
        "記録を停止",
        "Detener registro",
        "Interrompi registrazione",
        "Parar registro",
        "Hentikan pencatatan",
        "Arrêter la journalisation",
        "Aufzeichnung stoppen",
        "기록 중지",
        "Dừng ghi",
        "Kaydı durdur",
        "Остановить запись",
        "Zatrzymaj zapis";
    start_logging: "开始记录", "Start logging",
        "開始記錄",
        "記録を開始",
        "Iniciar registro",
        "Avvia registrazione",
        "Iniciar registro",
        "Mulai mencatat",
        "Démarrer la journalisation",
        "Aufzeichnung starten",
        "기록 시작",
        "Bắt đầu ghi",
        "Kaydı başlat",
        "Начать запись",
        "Rozpocznij zapis";
    pick_dir: "选择目录", "Choose dir",
        "選擇目錄",
        "フォルダを選択",
        "Elegir carpeta",
        "Scegli cartella",
        "Escolher pasta",
        "Pilih direktori",
        "Choisir un dossier",
        "Ordner wählen",
        "폴더 선택",
        "Chọn thư mục",
        "Klasör seç",
        "Выбрать папку",
        "Wybierz katalog";
    logged_bytes: "已记录 {} 字节", "{} bytes logged",
        "已記錄 {} 位元組",
        "{} バイト記録しました",
        "{} bytes registrados",
        "{} byte registrati",
        "{} bytes registrados",
        "{} bita tercatat",
        "{} octets enregistrés",
        "{} Bytes aufgezeichnet",
        "{} 바이트 기록됨",
        "Đã ghi {} byte",
        "{} bayt kaydedildi",
        "Записано {} байт",
        "Zapisano {} bajtów";
    not_logging: "未记录", "Not logging",
        "未記錄",
        "記録していません",
        "Sin registrar",
        "Non in registrazione",
        "Sem registro",
        "Tidak mencatat",
        "Pas de journalisation",
        "Keine Aufzeichnung",
        "기록 안 함",
        "Không ghi",
        "Kayıt yok",
        "Запись не ведётся",
        "Brak zapisu";
    output_dir_line: "输出目录: {}", "Output dir: {}",
        "輸出目錄: {}",
        "出力先: {}",
        "Carpeta de salida: {}",
        "Cartella di output: {}",
        "Pasta de saída: {}",
        "Direktori keluaran: {}",
        "Dossier de sortie : {}",
        "Ausgabeordner: {}",
        "출력 디렉터리: {}",
        "Thư mục đầu ra: {}",
        "Çıktı klasörü: {}",
        "Папка вывода: {}",
        "Katalog wyjściowy: {}";

    // —— panels/chart.rs：图表面板 ——
    chart_panel_label: "实时图表:", "Realtime chart:",
        "即時圖表:",
        "リアルタイムチャート:",
        "Gráfico en tiempo real:",
        "Grafico in tempo reale:",
        "Gráfico em tempo real:",
        "Grafik waktu nyata:",
        "Graphique en temps réel :",
        "Echtzeitdiagramm:",
        "실시간 차트:",
        "Biểu đồ thời gian thực:",
        "Canlı grafik:",
        "График в реальном времени:",
        "Wykres na żywo:";
    auto_bounds: "自动缩放", "Auto bounds",
        "自動縮放",
        "自動スケール",
        "Escala automática",
        "Scala automatica",
        "Escala automática",
        "Skala otomatis",
        "Échelle automatique",
        "Automatischer Maßstab",
        "자동 크기 조정",
        "Tự động điều chỉnh",
        "Otomatik sınırlar",
        "Автомасштаб",
        "Automatyczne granice";
    clear_chart: "清空图表", "Clear chart",
        "清空圖表",
        "チャートを消去",
        "Borrar gráfico",
        "Cancella grafico",
        "Limpar gráfico",
        "Bersihkan grafik",
        "Effacer le graphique",
        "Diagramm löschen",
        "차트 지우기",
        "Xóa biểu đồ",
        "Grafiği temizle",
        "Очистить график",
        "Wyczyść wykres";
    chart_summary:
        "{} 通道 · {} 样本 · 每通道上限 {}",
        "{} channels · {} samples · max {} each",
        "{} 通道 · {} 樣本 · 每通道上限 {}",
        "{} チャネル · {} サンプル · 各上限 {}",
        "{} canales · {} muestras · máx {} cada uno",
        "{} canali · {} campioni · max {} ciascuno",
        "{} canais · {} amostras · máx {} cada",
        "{} kanal · {} sampel · maks {} per kanal",
        "{} canaux · {} échantillons · max {} chacun",
        "{} Kanäle · {} Messwerte · max {} je Kanal",
        "{} 채널 · {} 샘플 · 채널당 최대 {}",
        "{} kênh · {} mẫu · tối đa {} mỗi kênh",
        "{} kanal · {} örnek · her biri en fazla {}",
        "{} каналов · {} значений · не более {} на канал",
        "{} kanałów · {} próbek · maks. {} na kanał";
    ignored_fields: "忽略 {} 个非数值字段", "ignored {} non-numeric fields",
        "忽略 {} 個非數值欄位",
        "非数値フィールドを {} 個無視",
        "se ignoraron {} campos no numéricos",
        "ignorati {} campi non numerici",
        "{} campos não numéricos ignorados",
        "mengabaikan {} bidang non-numerik",
        "{} champs non numériques ignorés",
        "{} nicht-numerische Felder ignoriert",
        "숫자가 아닌 필드 {}개 무시됨",
        "đã bỏ qua {} trường không phải số",
        "{} sayısal olmayan alan yok sayıldı",
        "пропущено {} нечисловых полей",
        "zignorowano {} pól nienumerycznych";
    discarded_segments: "丢弃 {} 段超长数据", "dropped {} oversized segments",
        "丟棄 {} 段超長資料",
        "長すぎるデータを {} 件破棄",
        "se descartaron {} segmentos demasiado largos",
        "scartati {} segmenti troppo lunghi",
        "{} segmentos grandes descartados",
        "membuang {} segmen yang terlalu panjang",
        "{} segments trop longs ignorés",
        "{} überlange Segmente verworfen",
        "너무 긴 데이터 {}개 버림",
        "đã bỏ {} đoạn quá dài",
        "{} aşırı uzun bölüm atıldı",
        "отброшено {} слишком длинных сегментов",
        "odrzucono {} zbyt długich segmentów";
    chart_format_hint:
        "{}（格式：一行一组样本，字段用逗号分隔，例如 1.0,2.0,3.0）",
        "{} (format: one sample group per line, comma-separated fields, e.g. 1.0,2.0,3.0)",
        "{}（格式：一行一組樣本，欄位用逗號分隔，例如 1.0,2.0,3.0）",
        "{}（形式: 1 行 1 組のサンプル、フィールドはカンマ区切り、例 1.0,2.0,3.0）",
        "{} (formato: un grupo de muestras por línea, campos separados por comas, p. ej. 1.0,2.0,3.0)",
        "{} (formato: un gruppo di campioni per riga, campi separati da virgole, es. 1.0,2.0,3.0)",
        "{} (formato: um grupo de amostras por linha, campos separados por vírgulas, ex. 1.0,2.0,3.0)",
        "{} (format: satu grup sampel per baris, bidang dipisah koma, mis. 1.0,2.0,3.0)",
        "{} (format : un groupe d'échantillons par ligne, champs séparés par des virgules, ex. 1.0,2.0,3.0)",
        "{} (Format: eine Messwertgruppe pro Zeile, Felder durch Kommas getrennt, z. B. 1.0,2.0,3.0)",
        "{} (형식: 한 줄에 샘플 한 그룹, 필드는 쉼표로 구분, 예: 1.0,2.0,3.0)",
        "{} (định dạng: một nhóm mẫu mỗi dòng, các trường cách nhau bằng dấu phẩy, ví dụ 1.0,2.0,3.0)",
        "{} (biçim: satır başına bir örnek grubu, alanlar virgülle ayrılır, örn. 1.0,2.0,3.0)",
        "{} (формат: одна группа значений в строке, поля через запятую, например 1.0,2.0,3.0)",
        "{} (format: jedna grupa próbek w wierszu, pola oddzielone przecinkami, np. 1.0,2.0,3.0)";
    waiting_data: "等待数据…", "Waiting for data…",
        "等待資料…",
        "データを待機中…",
        "Esperando datos…",
        "In attesa di dati…",
        "Aguardando dados…",
        "Menunggu data…",
        "En attente de données…",
        "Warte auf Daten…",
        "데이터 대기 중…",
        "Đang chờ dữ liệu…",
        "Veri bekleniyor…",
        "Ожидание данных…",
        "Oczekiwanie na dane…";

    // —— panels/menu_bar.rs：菜单 ——
    menu_file: "文件", "File",
        "檔案",
        "ファイル",
        "Archivo",
        "File",
        "Arquivo",
        "Berkas",
        "Fichier",
        "Datei",
        "파일",
        "Tệp",
        "Dosya",
        "Файл",
        "Plik";
    menu_info: "信息", "Info",
        "資訊",
        "情報",
        "Información",
        "Informazioni",
        "Informações",
        "Info",
        "Informations",
        "Info",
        "정보",
        "Thông tin",
        "Bilgi",
        "Информация",
        "Informacje";
    menu_quit: "退出", "Quit",
        "結束",
        "終了",
        "Salir",
        "Esci",
        "Sair",
        "Keluar",
        "Quitter",
        "Beenden",
        "종료",
        "Thoát",
        "Çık",
        "Выход",
        "Zakończ";
    settings_title: "设置", "Settings",
        "設定",
        "設定",
        "Ajustes",
        "Impostazioni",
        "Configurações",
        "Pengaturan",
        "Paramètres",
        "Einstellungen",
        "설정",
        "Cài đặt",
        "Ayarlar",
        "Настройки",
        "Ustawienia";
    menu_open_settings: "打开设置窗口", "Open settings window",
        "開啟設定視窗",
        "設定ウィンドウを開く",
        "Abrir ventana de ajustes",
        "Apri finestra impostazioni",
        "Abrir janela de configurações",
        "Buka jendela pengaturan",
        "Ouvrir la fenêtre des paramètres",
        "Einstellungsfenster öffnen",
        "설정 창 열기",
        "Mở cửa sổ cài đặt",
        "Ayarlar penceresini aç",
        "Открыть окно настроек",
        "Otwórz okno ustawień";
    theme_label: "主题", "Theme",
        "主題",
        "テーマ",
        "Tema",
        "Tema",
        "Tema",
        "Tema",
        "Thème",
        "Design",
        "테마",
        "Chủ đề",
        "Tema",
        "Тема",
        "Motyw";
    menu_view: "视图", "View",
        "檢視",
        "表示",
        "Ver",
        "Visualizza",
        "Exibir",
        "Tampilan",
        "Affichage",
        "Ansicht",
        "보기",
        "Xem",
        "Görünüm",
        "Вид",
        "Widok";
    panel_label: "面板", "Panels",
        "面板",
        "パネル",
        "Paneles",
        "Pannelli",
        "Painéis",
        "Panel",
        "Panneaux",
        "Bereiche",
        "패널",
        "Bảng điều khiển",
        "Paneller",
        "Панели",
        "Panele";
    display_mode_heading: "显示模式", "Display mode",
        "顯示模式",
        "表示モード",
        "Modo de vista",
        "Modalità vista",
        "Modo de exibição",
        "Mode tampilan",
        "Mode d'affichage",
        "Anzeigemodus",
        "표시 모드",
        "Chế độ hiển thị",
        "Görüntüleme modu",
        "Режим отображения",
        "Tryb wyświetlania";
    auto_scroll: "自动滚动", "Auto scroll",
        "自動捲動",
        "自動スクロール",
        "Desplazamiento automático",
        "Scorrimento automatico",
        "Rolagem automática",
        "Gulir otomatis",
        "Défilement automatique",
        "Automatisch scrollen",
        "자동 스크롤",
        "Tự động cuộn",
        "Otomatik kaydır",
        "Автопрокрутка",
        "Automatyczne przewijanie";
    always_on_top: "总在最前端", "Always on top",
        "總在最前端",
        "常在最前面顯示",
        "Siempre encima",
        "Sempre in primo piano",
        "Sempre no topo",
        "Selalu di atas",
        "Toujours au premier plan",
        "Immer im Vordergrund",
        "항상 위에 표시",
        "Luôn ở trên cùng",
        "Her zaman üstte",
        "Всегда поверх остальных",
        "Zawsze na wierzchu";
    force_wrap: "强制换行", "Force wrap",
        "強制換行",
        "強制折り返し",
        "Ajuste de línea forzado",
        "A capo forzato",
        "Quebra de linha forçada",
        "Ganti baris paksa",
        "Retour à la ligne forcé",
        "Zeilenumbruch erzwingen",
        "강제 줄바꿈",
        "Xuống dòng cưỡng bức",
        "Zorunlu satır sarma",
        "Принудительный перенос",
        "Wymuś zawijanie";
    force_wrap_hover:
        "长度超过 {} 个字符的行会被断开换行，更短的行保持原样；只作用于文本 / HEX 视图",
        "Lines longer than {} characters are broken onto several rows; shorter lines stay as they are. Text and HEX views only",
        "長度超過 {} 個字元的行會被斷開換行，更短的行保持原樣；只作用於文字 / HEX 檢視",
        "{} 文字を超える行は途中で折り返され、それより短い行はそのままです。対象はテキスト表示と HEX 表示のみ",
        "Las líneas de más de {} caracteres se parten en varias; las más cortas se quedan igual. Solo en las vistas de texto y HEX",
        "Le righe più lunghe di {} caratteri vengono spezzate su più righe; quelle più corte restano invariate. Solo per le viste testo e HEX",
        "Linhas com mais de {} caracteres são quebradas em várias; as mais curtas ficam iguais. Somente nas exibições de texto e HEX",
        "Baris lebih dari {} karakter dipotong menjadi beberapa baris; baris yang lebih pendek tetap apa adanya. Hanya untuk tampilan teks dan HEX",
        "Les lignes de plus de {} caractères sont coupées sur plusieurs lignes ; les plus courtes restent inchangées. Uniquement pour les vues texte et HEX",
        "Zeilen mit mehr als {} Zeichen werden umbrochen; kürzere bleiben unverändert. Nur für die Text- und HEX-Ansicht",
        "{} 자를 넘는 줄은 여러 줄로 나뉘고, 더 짧은 줄은 그대로 둡니다. 텍스트 / HEX 보기에만 적용됩니다",
        "Dòng dài hơn {} ký tự được ngắt xuống nhiều dòng; dòng ngắn hơn giữ nguyên. Chỉ áp dụng cho chế độ xem Văn bản / HEX",
        "{} karakterden uzun satırlar birden çok satıra bölünür; daha kısa satırlar olduğu gibi kalır. Yalnızca metin / HEX görünümünde geçerlidir",
        "Строки длиннее {} символов разбиваются на несколько строк, более короткие остаются как есть. Только для текстового и HEX-режима",
        "Wiersze dłuższe niż {} znaków są łamane na kilka wierszy; krótsze pozostają bez zmian. Dotyczy tylko widoku tekstowego i HEX";
    text: "文本", "Text",
        "文字",
        "テキスト",
        "Texto",
        "Testo",
        "Texto",
        "Teks",
        "Texte",
        "Text",
        "텍스트",
        "Văn bản",
        "Metin",
        "Текст",
        "Tekst";
    hex: "HEX", "HEX",
        "HEX",
        "HEX",
        "HEX",
        "HEX",
        "HEX",
        "HEX",
        "HEX",
        "HEX",
        "HEX",
        "HEX",
        "HEX",
        "HEX",
        "HEX";
    encoding_shared_label: "编码（接收与发送共用）", "Encoding (shared by receive and send)",
        "編碼（接收與傳送共用）",
        "エンコーディング（送受信で共通）",
        "Codificación (compartida por recepción y envío)",
        "Codifica (condivisa da ricezione e invio)",
        "Codificação (compartilhada por recepção e envio)",
        "Pengodean (dipakai bersama penerimaan dan pengiriman)",
        "Encodage (partagé par la réception et l'envoi)",
        "Kodierung (gemeinsam für Empfang und Senden)",
        "인코딩(수신과 송신 공용)",
        "Mã hóa (dùng chung cho nhận và gửi)",
        "Kodlama (alım ve gönderim ortak)",
        "Кодировка (общая для приёма и отправки)",
        "Kodowanie (wspólne dla odbioru i wysyłania)";
    encoding_gbk_note_menu:
        "GBK / GB18030 接收结果相同，仅发送时的可表示字符范围不同",
        "GBK and GB18030 decode identically; only the encodable character range differs",
        "GBK / GB18030 接收結果相同，僅傳送時的可表示字元範圍不同",
        "GBK / GB18030 の受信結果は同じで、送信時に表現できる文字範囲だけが異なります",
        "GBK y GB18030 decodifican igual; solo cambia el rango de caracteres codificables al enviar",
        "GBK e GB18030 decodificano allo stesso modo; cambia solo l'intervallo di caratteri codificabili all'invio",
        "GBK e GB18030 decodificam igual; só muda o intervalo de caracteres codificáveis no envio",
        "GBK dan GB18030 mendekode sama; hanya rentang karakter yang dapat dikodekan saat mengirim yang berbeda",
        "GBK et GB18030 décodent à l'identique ; seule la plage de caractères encodables à l'envoi diffère",
        "GBK und GB18030 dekodieren identisch; nur der beim Senden darstellbare Zeichenbereich unterscheidet sich",
        "GBK / GB18030의 수신 결과는 같고, 송신 시 표현 가능한 문자 범위만 다릅니다",
        "GBK / GB18030 giải mã giống nhau; chỉ khác phạm vi ký tự có thể mã hóa khi gửi",
        "GBK / GB18030 aynı şekilde çözer; yalnızca gönderirken kodlanabilen karakter aralığı farklıdır",
        "GBK и GB18030 декодируются одинаково; различается только диапазон символов, кодируемых при отправке",
        "GBK i GB18030 dekodują identycznie; różni się tylko zakres znaków możliwych do zakodowania przy wysyłaniu";
    send_title: "发送", "Send",
        "傳送",
        "送信",
        "Enviar",
        "Invia",
        "Enviar",
        "Kirim",
        "Envoyer",
        "Senden",
        "전송",
        "Gửi",
        "Gönder",
        "Отправка",
        "Wyślij";
    hex_receive: "HEX 接收", "HEX receive",
        "HEX 接收",
        "HEX 受信",
        "Recepción HEX",
        "Ricezione HEX",
        "Recepção HEX",
        "Penerimaan HEX",
        "Réception HEX",
        "HEX-Empfang",
        "HEX 수신",
        "Nhận HEX",
        "HEX alım",
        "HEX-приём",
        "Odbiór HEX";
    nav_receive: "接收区", "Receive",
        "接收區",
        "受信エリア",
        "Recepción",
        "Ricezione",
        "Recepção",
        "Penerimaan",
        "Réception",
        "Empfang",
        "수신 영역",
        "Vùng nhận",
        "Alım",
        "Приём",
        "Odbiór";
    nav_chart: "实时图表", "Realtime chart",
        "即時圖表",
        "リアルタイムチャート",
        "Gráfico en tiempo real",
        "Grafico in tempo reale",
        "Gráfico em tempo real",
        "Grafik waktu nyata",
        "Graphique en temps réel",
        "Echtzeitdiagramm",
        "실시간 차트",
        "Biểu đồ thời gian thực",
        "Canlı grafik",
        "График в реальном времени",
        "Wykres na żywo";
    nav_send: "发送区", "Send",
        "傳送區",
        "送信エリア",
        "Envío",
        "Invio",
        "Envio",
        "Kirim",
        "Envoi",
        "Senden",
        "송신 영역",
        "Vùng gửi",
        "Gönderme",
        "Отправка",
        "Wysyłanie";
    at_least_one_panel: "至少保留一个面板", "At least one panel must stay visible",
        "至少保留一個面板",
        "パネルは最低 1 つ表示してください",
        "Debe quedar visible al menos un panel",
        "Almeno un pannello deve restare visibile",
        "Ao menos um painel deve permanecer visível",
        "Setidaknya satu panel harus tetap terlihat",
        "Au moins un panneau doit rester visible",
        "Mindestens ein Bereich muss sichtbar bleiben",
        "패널을 최소 하나는 표시해야 합니다",
        "Phải giữ ít nhất một bảng điều khiển hiển thị",
        "En az bir panel görünür kalmalı",
        "Хотя бы одна панель должна остаться видимой",
        "Co najmniej jeden panel musi pozostać widoczny";
    theme_system: "跟随系统", "System",
        "跟隨系統",
        "システムに従う",
        "Sistema",
        "Sistema",
        "Sistema",
        "Sistem",
        "Système",
        "System",
        "시스템 설정",
        "Theo hệ thống",
        "Sistem",
        "Системная",
        "Systemowy";
    theme_light: "浅色", "Light",
        "淺色",
        "ライト",
        "Claro",
        "Chiaro",
        "Claro",
        "Terang",
        "Clair",
        "Hell",
        "라이트",
        "Sáng",
        "Açık",
        "Светлая",
        "Jasny";
    theme_dark: "深色", "Dark",
        "深色",
        "ダーク",
        "Oscuro",
        "Scuro",
        "Escuro",
        "Gelap",
        "Sombre",
        "Dunkel",
        "다크",
        "Tối",
        "Koyu",
        "Тёмная",
        "Ciemny";
    ts_off: "关闭", "Off",
        "關閉",
        "オフ",
        "Desactivado",
        "Disattivato",
        "Desativado",
        "Nonaktif",
        "Désactivé",
        "Aus",
        "끔",
        "Tắt",
        "Kapalı",
        "Выкл.",
        "Wył.";
    ts_per_line: "每行", "Per line",
        "每行",
        "行ごと",
        "Por línea",
        "Per riga",
        "Por linha",
        "Per baris",
        "Par ligne",
        "Pro Zeile",
        "줄마다",
        "Mỗi dòng",
        "Satır başına",
        "На строку",
        "Na wiersz";
    ts_per_batch: "每批", "Per batch",
        "每批",
        "バッチごと",
        "Por lote",
        "Per blocco",
        "Por lote",
        "Per batch",
        "Par lot",
        "Pro Block",
        "배치마다",
        "Mỗi lô",
        "Blok başına",
        "На блок",
        "Na blok";
    ending_none: "无", "None",
        "無",
        "なし",
        "Ninguno",
        "Nessuno",
        "Nenhum",
        "Tidak ada",
        "Aucun",
        "Keine",
        "없음",
        "Không",
        "Yok",
        "Нет",
        "Brak";
    ending_lf: "LF", "LF",
        "LF",
        "LF",
        "LF",
        "LF",
        "LF",
        "LF",
        "LF",
        "LF",
        "LF",
        "LF",
        "LF",
        "LF",
        "LF";
    ending_crlf: "CRLF", "CRLF",
        "CRLF",
        "CRLF",
        "CRLF",
        "CRLF",
        "CRLF",
        "CRLF",
        "CRLF",
        "CRLF",
        "CRLF",
        "CRLF",
        "CRLF",
        "CRLF",
        "CRLF";

    // —— panels/menu_bar.rs：发送历史 ——
    send_history_label: "发送历史", "Send history",
        "傳送歷史",
        "送信履歴",
        "Historial de envíos",
        "Cronologia invii",
        "Histórico de envios",
        "Riwayat pengiriman",
        "Historique d'envoi",
        "Sende-Verlauf",
        "전송 기록",
        "Lịch sử gửi",
        "Gönderme geçmişi",
        "История отправки",
        "Historia wysyłania";
    no_send_history: "还没有发送记录", "No sends yet",
        "還沒有傳送記錄",
        "送信履歴はまだありません",
        "Aún no hay envíos",
        "Nessun invio ancora",
        "Ainda não há envios",
        "Belum ada pengiriman",
        "Aucun envoi pour l'instant",
        "Noch keine Sendungen",
        "전송 기록이 없습니다",
        "Chưa có lần gửi nào",
        "Henüz gönderim yok",
        "Отправок пока нет",
        "Brak wysłanych danych";
    send_history_count: "发送历史（{} / {}）", "Send history ({} / {})",
        "傳送歷史（{} / {}）",
        "送信履歴（{} / {}）",
        "Historial de envíos ({} / {})",
        "Cronologia invii ({} / {})",
        "Histórico de envios ({} / {})",
        "Riwayat pengiriman ({} / {})",
        "Historique d'envoi ({} / {})",
        "Sende-Verlauf ({} / {})",
        "전송 기록 ({} / {})",
        "Lịch sử gửi ({} / {})",
        "Gönderme geçmişi ({} / {})",
        "История отправки ({} / {})",
        "Historia wysyłania ({} / {})";
    empty_line: "<空行>", "<empty line>",
        "<空行>",
        "<空行>",
        "<línea vacía>",
        "<riga vuota>",
        "<linha vazia>",
        "<baris kosong>",
        "<ligne vide>",
        "<Leerzeile>",
        "<빈 줄>",
        "<dòng trống>",
        "<boş satır>",
        "<пустая строка>",
        "<pusty wiersz>";
    clear_history: "清空历史", "Clear history",
        "清空歷史",
        "履歴を消去",
        "Borrar historial",
        "Cancella cronologia",
        "Limpar histórico",
        "Bersihkan riwayat",
        "Effacer l'historique",
        "Verlauf löschen",
        "기록 지우기",
        "Xóa lịch sử",
        "Geçmişi temizle",
        "Очистить историю",
        "Wyczyść historię";

    // —— panels/settings.rs：界面 ——
    sec_interface: "界面", "Interface",
        "介面",
        "インターフェース",
        "Interfaz",
        "Interfaccia",
        "Interface",
        "Antarmuka",
        "Interface",
        "Oberfläche",
        "인터페이스",
        "Giao diện",
        "Arayüz",
        "Интерфейс",
        "Interfejs";
    language_label: "语言", "Language",
        "語言",
        "言語",
        "Idioma",
        "Lingua",
        "Idioma",
        "Bahasa",
        "Langue",
        "Sprache",
        "언어",
        "Ngôn ngữ",
        "Dil",
        "Язык",
        "Język";
    language_applies_immediately:
        "切换语言立即生效，不需要重启",
        "Switching language takes effect immediately, no restart needed",
        "切換語言立即生效，不需要重啟",
        "言語の切り替えはすぐに反映され、再起動は不要です",
        "El cambio de idioma se aplica de inmediato, sin reiniciar",
        "Il cambio di lingua ha effetto immediato, senza riavvio",
        "A troca de idioma entra em vigor imediatamente, sem reiniciar",
        "Pergantian bahasa langsung berlaku, tanpa perlu mulai ulang",
        "Le changement de langue prend effet immédiatement, sans redémarrage",
        "Der Sprachwechsel wirkt sofort, kein Neustart nötig",
        "언어 변경은 즉시 적용되며 재시작이 필요 없습니다",
        "Đổi ngôn ngữ có hiệu lực ngay, không cần khởi động lại",
        "Dil değişikliği hemen geçerli olur, yeniden başlatma gerekmez",
        "Смена языка применяется сразу, перезапуск не нужен",
        "Zmiana języka działa natychmiast, bez ponownego uruchamiania";
    font_scale_label: "字体大小", "Font size",
        "字型大小",
        "文字サイズ",
        "Tamaño de fuente",
        "Dimensione carattere",
        "Tamanho da fonte",
        "Ukuran huruf",
        "Taille de police",
        "Schriftgröße",
        "글꼴 크기",
        "Cỡ chữ",
        "Yazı tipi boyutu",
        "Размер шрифта",
        "Rozmiar czcionki";
    font_scale_small: "小", "Small",
        "小",
        "小",
        "Pequeño",
        "Piccolo",
        "Pequeno",
        "Kecil",
        "Petite",
        "Klein",
        "작게",
        "Nhỏ",
        "Küçük",
        "Мелкий",
        "Małe";
    font_scale_normal: "标准", "Normal",
        "標準",
        "標準",
        "Normal",
        "Normale",
        "Normal",
        "Normal",
        "Normale",
        "Normal",
        "보통",
        "Tiêu chuẩn",
        "Normal",
        "Обычный",
        "Standardowe";
    font_scale_large: "大", "Large",
        "大",
        "大",
        "Grande",
        "Grande",
        "Grande",
        "Besar",
        "Grande",
        "Groß",
        "크게",
        "Lớn",
        "Büyük",
        "Крупный",
        "Duże";
    font_scale_xlarge: "特大", "Extra large",
        "特大",
        "特大",
        "Muy grande",
        "Molto grande",
        "Muito grande",
        "Sangat besar",
        "Très grande",
        "Sehr groß",
        "아주 크게",
        "Rất lớn",
        "Çok büyük",
        "Очень крупный",
        "Bardzo duże";
    font_scale_note:
        "整体缩放界面文字（标题、正文、按钮、等宽一起变），立即生效；接收区与发送框的等宽字号也跟着变。\
         若调到「特大」后某些标签显得拥挤，属已知情况——它遇到的是界面里固定像素的标签列宽度",
        "Scales all interface text together (headings, body, buttons, monospace), effective immediately; \
         the monospace size in the receive area and send box follows too. If some labels look cramped at \
         \"Extra large\", that is expected — they run into the fixed-pixel label column widths",
        "整體縮放介面文字（標題、內文、按鈕、等寬一起變），立即生效；接收區與傳送框的等寬字號也跟著變。若調到「特大」後某些標籤顯得擁擠，屬已知情況——它遇到的是介面裡固定像素的標籤欄寬度",
        "UI 文字をまとめて拡大縮小します（見出し・本文・ボタン・等幅も一緒に変化）。すぐ反映され、受信エリアと送信欄の等幅サイズも連動します。「特大」にすると一部のラベルが窮屈に見えるのは既知の挙動です——固定ピクセルのラベル列幅に当たっているためです",
        "Escala todo el texto de la interfaz (títulos, cuerpo, botones y monoespaciado) a la vez, con efecto inmediato; el tamaño monoespaciado del área de recepción y del cuadro de envío también cambia. Si algunas etiquetas se ven apretadas en \"Muy grande\", es esperado: chocan con el ancho fijo en píxeles de la columna de etiquetas",
        "Ridimensiona insieme tutto il testo dell'interfaccia (titoli, corpo, pulsanti, monospaziato), con effetto immediato; anche la dimensione monospaziata nell'area di ricezione e nella casella di invio cambia. Se alcune etichette appaiono strette con \"Molto grande\", è previsto: urtano contro la larghezza fissa in pixel della colonna delle etichette",
        "Escala todo o texto da interface (títulos, corpo, botões e monoespaçado) em conjunto, com efeito imediato; o tamanho monoespaçado na área de recepção e na caixa de envio também muda. Se alguns rótulos ficarem apertados em \"Muito grande\", é esperado: eles esbarram na largura fixa em pixels da coluna de rótulos",
        "Menskalakan seluruh teks antarmuka sekaligus (judul, isi, tombol, monospace), langsung berlaku; ukuran monospace di area penerimaan dan kotak kirim juga ikut berubah. Jika beberapa label tampak sesak pada \"Sangat besar\", itu wajar — label menabrak lebar kolom label yang tetap dalam piksel",
        "Met à l'échelle tout le texte de l'interface (titres, corps, boutons, monospace) d'un coup, effet immédiat ; la taille monospace de la zone de réception et du champ d'envoi suit aussi. Si certaines étiquettes paraissent à l'étroit en « Très grande », c'est attendu : elles se heurtent à la largeur fixe en pixels de la colonne d'étiquettes",
        "Skaliert den gesamten Oberflächentext gemeinsam (Überschriften, Fließtext, Schaltflächen, Monospace), sofort wirksam; auch die Monospace-Größe im Empfangsbereich und im Sendefeld ändert sich. Wirken einige Beschriftungen bei \"Sehr groß\" gedrängt, ist das erwartet — sie stoßen an die feste Pixelbreite der Beschriftungsspalte",
        "인터페이스 텍스트 전체를 함께 확대·축소합니다(제목, 본문, 버튼, 고정폭 포함). 즉시 적용되며 수신 영역과 송신 상자의 고정폭 크기도 함께 바뀝니다. \"아주 크게\"에서 일부 라벨이 비좁아 보이는 것은 알려진 현상입니다——고정 픽셀 라벨 열 너비에 부딪히기 때문입니다",
        "Phóng to/thu nhỏ toàn bộ chữ trong giao diện cùng lúc (tiêu đề, nội dung, nút, chữ đẳng khoảng), có hiệu lực ngay; cỡ chữ đẳng khoảng ở vùng nhận và ô gửi cũng thay đổi theo. Nếu vài nhãn trông chật ở mức \"Rất lớn\", đó là điều đã biết — chúng chạm giới hạn chiều rộng cột nhãn cố định theo pixel",
        "Tüm arayüz metnini birlikte ölçekler (başlıklar, gövde, düğmeler, eş aralıklı), hemen geçerli olur; alım alanı ile gönderme kutusunun eş aralıklı boyutu da buna uyar. «Çok büyük» seçildiğinde bazı etiketlerin sıkışık görünmesi beklenen bir durumdur — sabit pikselli etiket sütunu genişliğine dayanırlar",
        "Масштабирует весь текст интерфейса сразу (заголовки, основной текст, кнопки, моноширинный), применяется немедленно; размер моноширинного шрифта в области приёма и поле отправки тоже меняется. Если при «Очень крупном» некоторые подписи выглядят тесно, это ожидаемо — они упираются в фиксированную ширину столбца подписей",
        "Skaluje cały tekst interfejsu naraz (nagłówki, treść, przyciski, czcionkę o stałej szerokości), działa natychmiast; rozmiar czcionki o stałej szerokości w obszarze odbioru i polu wysyłania też się zmienia. Jeśli przy „Bardzo dużym” niektóre etykiety wydają się ciasne, jest to oczekiwane — napotykają stałą szerokość kolumny etykiet w pikselach";
    font_scale_preview: "预览：{}", "Preview: {}",
        "預覽：{}",
        "プレビュー: {}",
        "Vista previa: {}",
        "Anteprima: {}",
        "Pré-visualização: {}",
        "Pratinjau: {}",
        "Aperçu : {}",
        "Vorschau: {}",
        "미리보기: {}",
        "Xem trước: {}",
        "Önizleme: {}",
        "Предпросмотр: {}",
        "Podgląd: {}";

    // —— panels/settings.rs：文本编码 ——
    sec_text_encoding: "文本编码", "Text encoding",
        "文字編碼",
        "テキストエンコーディング",
        "Codificación de texto",
        "Codifica testo",
        "Codificação de texto",
        "Pengodean teks",
        "Encodage du texte",
        "Textkodierung",
        "텍스트 인코딩",
        "Mã hóa văn bản",
        "Kodlama",
        "Кодировка текста",
        "Kodowanie tekstu";
    encoding_label: "编码", "Encoding",
        "編碼",
        "エンコーディング",
        "Codificación",
        "Codifica",
        "Codificação",
        "Pengodean",
        "Encodage",
        "Kodierung",
        "인코딩",
        "Mã hóa",
        "Kodlama",
        "Кодировка",
        "Kodowanie";
    encoding_shared_note:
        "接收解码与发送编码共用这一档：不会出现「收到的中文按 GBK 解出来、发出去的却是 UTF-8」",
        "Receive decoding and send encoding share this setting: you will never decode incoming Chinese as GBK while sending out UTF-8",
        "接收解碼與傳送編碼共用這一檔：不會出現「收到的中文按 GBK 解出來、傳出去的卻是 UTF-8」",
        "受信デコードと送信エンコードはこの設定を共有します。「受信した中国語を GBK で解釈し、送信は UTF-8」という食い違いは起きません",
        "La decodificación de recepción y la codificación de envío comparten este ajuste: nunca decodificará chino entrante como GBK mientras envía UTF-8",
        "La decodifica in ricezione e la codifica in invio condividono questa impostazione: non accadrà mai di decodificare il cinese in arrivo come GBK inviando UTF-8",
        "A decodificação da recepção e a codificação do envio compartilham esta configuração: nunca vai decodificar chinês recebido como GBK enquanto envia UTF-8",
        "Dekode penerimaan dan kode pengiriman berbagi pengaturan ini: tidak akan pernah mendekode teks Tionghoa masuk sebagai GBK sementara mengirim UTF-8",
        "Le décodage en réception et l'encodage à l'envoi partagent ce réglage : vous ne décoderez jamais du chinois reçu en GBK tout en envoyant de l'UTF-8",
        "Empfangsdekodierung und Sende-Kodierung teilen diese Einstellung: Es passiert nie, dass eingehendes Chinesisch als GBK dekodiert wird, während UTF-8 gesendet wird",
        "수신 디코딩과 송신 인코딩이 이 설정을 공유합니다: \"받은 중국어를 GBK로 해석하고 보내는 것은 UTF-8\" 같은 불일치가 생기지 않습니다",
        "Giải mã khi nhận và mã hóa khi gửi dùng chung thiết lập này: sẽ không xảy ra việc giải mã chữ Trung nhận được theo GBK trong khi gửi bằng UTF-8",
        "Alım çözme ve gönderme kodlama bu ayarı paylaşır: gelen Çinceyi GBK ile çözüp UTF-8 gönderme durumu oluşmaz",
        "Декодирование приёма и кодирование отправки используют одну настройку: не возникнет ситуации, когда входящий текст декодируется как GBK, а отправляется как UTF-8",
        "Dekodowanie odbioru i kodowanie wysyłania korzystają z tego samego ustawienia: nie zdarzy się, że przychodzący tekst jest dekodowany jako GBK, a wysyłany jako UTF-8";
    encoding_gbk_note:
        "GBK 与 GB18030 的接收结果相同（共用解码器），仅发送时的可表示字符范围不同",
        "GBK and GB18030 decode identically (same decoder); only the encodable character range differs",
        "GBK 與 GB18030 的接收結果相同（共用解碼器），僅傳送時的可表示字元範圍不同",
        "GBK と GB18030 の受信結果は同じです（デコーダを共用）。送信時に表現できる文字範囲だけが異なります",
        "GBK y GB18030 decodifican igual (mismo decodificador); solo difiere el rango de caracteres codificables al enviar",
        "GBK e GB18030 decodificano allo stesso modo (stesso decodificatore); differisce solo l'intervallo di caratteri codificabili all'invio",
        "GBK e GB18030 decodificam igual (mesmo decodificador); só difere o intervalo de caracteres codificáveis no envio",
        "GBK dan GB18030 mendekode sama (dekoder yang sama); hanya rentang karakter yang dapat dikodekan saat mengirim yang berbeda",
        "GBK et GB18030 décodent à l'identique (même décodeur) ; seule diffère la plage de caractères encodables à l'envoi",
        "GBK und GB18030 dekodieren identisch (gleicher Decoder); nur der beim Senden darstellbare Zeichenbereich unterscheidet sich",
        "GBK와 GB18030의 수신 결과는 같습니다(디코더 공용). 송신 시 표현 가능한 문자 범위만 다릅니다",
        "GBK và GB18030 giải mã giống nhau (dùng chung bộ giải mã); chỉ khác phạm vi ký tự có thể mã hóa khi gửi",
        "GBK ile GB18030 aynı şekilde çözer (ortak çözücü); yalnızca gönderirken kodlanabilen karakter aralığı farklıdır",
        "GBK и GB18030 декодируются одинаково (общий декодер); различается только диапазон символов, кодируемых при отправке",
        "GBK i GB18030 dekodują identycznie (wspólny dekoder); różni się tylko zakres znaków możliwych do zakodowania przy wysyłaniu";
    encoding_switch_note:
        "切换到别的编码会丢弃上一批残留的半个字符，已经显示的内容不受影响",
        "Switching encoding discards a half character left over from the previous batch; text already shown is unaffected",
        "切換到別的編碼會丟棄上一批殘留的半個字元，已經顯示的內容不受影響",
        "別のエンコーディングに切り替えると、前のバッチで残った半端な文字は破棄されます。すでに表示済みの内容には影響しません",
        "Cambiar de codificación descarta el medio carácter sobrante del lote anterior; el contenido ya mostrado no se ve afectado",
        "Cambiando codifica si scarta il mezzo carattere rimasto dal blocco precedente; il testo già mostrato non è interessato",
        "Trocar a codificação descarta o meio caractere restante do lote anterior; o conteúdo já exibido não é afetado",
        "Mengganti pengodean membuang setengah karakter sisa dari batch sebelumnya; konten yang sudah ditampilkan tidak terpengaruh",
        "Changer d'encodage rejette le demi-caractère restant du lot précédent ; le contenu déjà affiché n'est pas affecté",
        "Ein Wechsel der Kodierung verwirft das halbe Zeichen aus dem vorherigen Block; bereits angezeigter Inhalt bleibt unberührt",
        "다른 인코딩으로 바꾸면 이전 배치에 남은 반쪽 문자를 버립니다. 이미 표시된 내용은 영향을 받지 않습니다",
        "Chuyển sang mã hóa khác sẽ bỏ ký tự nửa vời còn lại của lô trước; nội dung đã hiển thị không bị ảnh hưởng",
        "Başka bir kodlamaya geçmek önceki bloktan kalan yarım karakteri atar; hâlihazırda görünen içerik etkilenmez",
        "Переключение кодировки отбрасывает половинку символа, оставшуюся от предыдущего блока; уже показанное содержимое не затрагивается",
        "Przełączenie kodowania odrzuca połowę znaku pozostałą z poprzedniego bloku; już wyświetlona treść nie zmienia się";

    // —— panels/settings.rs：接收 ——
    sec_receive: "接收", "Receive",
        "接收",
        "受信",
        "Recepción",
        "Ricezione",
        "Recepção",
        "Penerimaan",
        "Réception",
        "Empfang",
        "수신",
        "Nhận",
        "Alım",
        "Приём",
        "Odbiór";
    timestamp_prefix_field: "时间戳前缀", "Timestamp prefix",
        "時間戳前綴",
        "タイムスタンプ接頭辞",
        "Prefijo de marca de tiempo",
        "Prefisso timestamp",
        "Prefixo de carimbo de tempo",
        "Awalan stempel waktu",
        "Préfixe d'horodatage",
        "Zeitstempel-Präfix",
        "타임스탬프 접두사",
        "Tiền tố dấu thời gian",
        "Zaman damgası öneki",
        "Префикс метки времени",
        "Prefiks znacznika czasu";
    ts_note:
        "形如 [12:00:00.123] 的本地时间前缀；每行 = 每个换行后重新标注，每批 = 每次收到数据标注一次",
        "Local time prefix such as [12:00:00.123]; per line = re-stamped after every newline, per batch = stamped once per received chunk",
        "形如 [12:00:00.123] 的本機時間前綴；每行 = 每個換行後重新標註，每批 = 每次收到資料標註一次",
        "[12:00:00.123] のようなローカル時刻の接頭辞。行ごと = 改行のたびに付け直し、バッチごと = 受信のたびに 1 回付けます",
        "Prefijo de hora local como [12:00:00.123]; por línea = se vuelve a marcar tras cada salto de línea, por lote = se marca una vez por cada bloque recibido",
        "Prefisso di ora locale come [12:00:00.123]; per riga = rimarcato dopo ogni a capo, per blocco = marcato una volta per ogni blocco ricevuto",
        "Prefixo de hora local como [12:00:00.123]; por linha = remarcado após cada quebra de linha, por lote = marcado uma vez por bloco recebido",
        "Awalan waktu lokal seperti [12:00:00.123]; per baris = ditandai ulang setiap baris baru, per batch = ditandai sekali per potongan yang diterima",
        "Préfixe d'heure locale tel que [12:00:00.123] ; par ligne = ré-apposé après chaque saut de ligne, par lot = apposé une fois par bloc reçu",
        "Lokaler Zeitpräfix wie [12:00:00.123]; pro Zeile = nach jedem Zeilenumbruch neu gesetzt, pro Block = einmal pro empfangenem Block gesetzt",
        "[12:00:00.123] 형태의 로컬 시간 접두사. 줄마다 = 줄바꿈마다 다시 표시, 배치마다 = 데이터를 받을 때마다 한 번 표시",
        "Tiền tố giờ cục bộ dạng [12:00:00.123]; mỗi dòng = đánh dấu lại sau mỗi lần xuống dòng, mỗi lô = đánh dấu một lần cho mỗi gói nhận được",
        "[12:00:00.123] biçiminde yerel saat öneki; satır başına = her satır sonunda yeniden damgalanır, blok başına = alınan her parça için bir kez",
        "Префикс местного времени вида [12:00:00.123]; на строку = заново после каждого перевода строки, на блок = один раз на каждый принятый блок",
        "Prefiks czasu lokalnego w postaci [12:00:00.123]; na wiersz = ponownie po każdym przejściu do nowego wiersza, na blok = raz na każdy odebrany fragment";
    ts_note_2:
        "前缀是显示注解而不是收到的数据：它会随接收区一起进导出文件与实时日志，但不会进入图表解析",
        "The prefix is a display annotation, not received data: it enters exports and the live log along with the receive area, but never chart parsing",
        "前綴是顯示註解而不是收到的資料：它會隨接收區一起進匯出檔與即時日誌，但不會進入圖表解析",
        "接頭辞は受信データではなく表示上の注釈です。受信エリアと一緒に書き出しファイルとリアルタイムログに入りますが、チャート解析には入りません",
        "El prefijo es una anotación visual, no datos recibidos: entra en las exportaciones y en el registro en vivo junto con el área de recepción, pero nunca en el análisis del gráfico",
        "Il prefisso è un'annotazione visiva, non un dato ricevuto: entra nelle esportazioni e nel log in tempo reale insieme all'area di ricezione, ma mai nell'analisi del grafico",
        "O prefixo é uma anotação de exibição, não dados recebidos: entra nas exportações e no log ao vivo junto com a área de recepção, mas nunca na análise do gráfico",
        "Awalan adalah anotasi tampilan, bukan data yang diterima: ikut masuk ke ekspor dan log langsung bersama area penerimaan, tetapi tidak pernah ke analisis grafik",
        "Le préfixe est une annotation d'affichage, pas une donnée reçue : il entre dans les exports et le journal en direct avec la zone de réception, mais jamais dans l'analyse du graphique",
        "Das Präfix ist eine Anzeige-Annotation, keine empfangenen Daten: Es gelangt zusammen mit dem Empfangsbereich in Exporte und Live-Protokoll, aber nie in die Diagrammauswertung",
        "접두사는 수신 데이터가 아니라 표시용 주석입니다. 수신 영역과 함께 내보내기 파일과 실시간 로그에 들어가지만 차트 분석에는 들어가지 않습니다",
        "Tiền tố là chú thích hiển thị, không phải dữ liệu nhận được: nó đi cùng vùng nhận vào tệp xuất và log trực tiếp, nhưng không bao giờ vào phân tích biểu đồ",
        "Önek alınan veri değil, görüntüleme notudur: alım alanıyla birlikte dışa aktarma dosyasına ve canlı günlüğe girer, ancak grafik ayrıştırmasına hiç girmez",
        "Префикс — это отображаемая пометка, а не принятые данные: он попадает в экспорт и в живой журнал вместе с областью приёма, но никогда не участвует в разборе графика",
        "Prefiks to adnotacja wyświetlania, a nie odebrane dane: trafia do eksportu i dziennika na żywo razem z obszarem odbioru, ale nigdy do analizy wykresu";
    max_log_field: "日志长度上限", "Max log length",
        "日誌長度上限",
        "ログ長の上限",
        "Longitud máxima del registro",
        "Lunghezza massima del log",
        "Comprimento máximo do log",
        "Batas panjang log",
        "Longueur maximale du journal",
        "Maximale Protokolllänge",
        "로그 최대 길이",
        "Giới hạn độ dài log",
        "Günlük uzunluğu üst sınırı",
        "Предел длины журнала",
        "Limit długości dziennika";
    chars_unlimited: "字符（0 = 不限）", "characters (0 = unlimited)",
        "字元（0 = 不限）",
        "文字（0 = 無制限）",
        "caracteres (0 = sin límite)",
        "caratteri (0 = illimitato)",
        "caracteres (0 = ilimitado)",
        "karakter (0 = tanpa batas)",
        "caractères (0 = illimité)",
        "Zeichen (0 = unbegrenzt)",
        "문자(0 = 무제한)",
        "ký tự (0 = không giới hạn)",
        "karakter (0 = sınırsız)",
        "символов (0 = без ограничения)",
        "znaków (0 = bez limitu)";
    max_log_note:
        "超出上限时从最旧的内容开始淘汰，三种接收视图同步裁剪",
        "When over the limit the oldest content is dropped; all three receive views are trimmed together",
        "超出上限時從最舊的內容開始淘汰，三種接收檢視同步裁剪",
        "上限を超えると古い内容から破棄され、3 つの受信ビューが同時に切り詰められます",
        "Al superar el límite se descarta lo más antiguo; las tres vistas de recepción se recortan a la vez",
        "Superato il limite, si scarta prima il contenuto più vecchio; le tre viste di ricezione vengono ridotte insieme",
        "Ao ultrapassar o limite, o conteúdo mais antigo é descartado; as três vistas de recepção são cortadas juntas",
        "Jika melebihi batas, konten terlama dibuang; ketiga tampilan penerimaan dipangkas bersamaan",
        "Au-delà de la limite, le contenu le plus ancien est supprimé ; les trois vues de réception sont tronquées ensemble",
        "Über dem Limit wird der älteste Inhalt verworfen; alle drei Empfangsansichten werden gemeinsam gekürzt",
        "상한을 넘으면 가장 오래된 내용부터 버려지고, 세 가지 수신 보기가 함께 잘립니다",
        "Khi vượt giới hạn, nội dung cũ nhất bị loại bỏ; cả ba chế độ xem nhận được cắt đồng thời",
        "Üst sınır aşıldığında en eski içerik atılır; üç alım görünümü birlikte kırpılır",
        "При превышении предела отбрасывается самое старое содержимое; все три вида отображения приёма обрезаются вместе",
        "Po przekroczeniu limitu odrzucana jest najstarsza treść; wszystkie trzy widoki odbioru są przycinane jednocześnie";
    auto_scroll_latest: "自动滚动到最新", "Auto scroll to newest",
        "自動捲動到最新",
        "最新まで自動スクロール",
        "Desplazar a lo más reciente",
        "Scorri all'ultimo",
        "Rolar até o mais recente",
        "Gulir ke terbaru",
        "Défiler vers le plus récent",
        "Zum Neuesten scrollen",
        "최신으로 자동 스크롤",
        "Tự động cuộn đến mới nhất",
        "En yeniye otomatik kaydır",
        "Автопрокрутка к новому",
        "Przewijaj do najnowszego";
    repaint_field: "重绘间隔", "Repaint interval",
        "重繪間隔",
        "再描画間隔",
        "Intervalo de repintado",
        "Intervallo di ridisegno",
        "Intervalo de repintura",
        "Interval penggambaran ulang",
        "Intervalle de rafraîchissement",
        "Neuzeichnungsintervall",
        "다시 그리기 간격",
        "Khoảng vẽ lại",
        "Yeniden çizim aralığı",
        "Интервал перерисовки",
        "Interwał odświeżania";
    repaint_note:
        "ms（16~1000，越小越跟手也越耗电）",
        "ms (16~1000; smaller is snappier and uses more power)",
        "ms（16~1000，越小越跟手也越耗電）",
        "ms（16〜1000、小さいほど追従が良く消費電力も増えます）",
        "ms (16~1000; menor es más ágil y consume más)",
        "ms (16~1000; più piccolo è più reattivo e consuma di più)",
        "ms (16~1000; menor é mais responsivo e consome mais)",
        "ms (16~1000; makin kecil makin responsif dan makin boros daya)",
        "ms (16~1000 ; plus petit = plus réactif et plus énergivore)",
        "ms (16~1000; kleiner ist flüssiger und verbraucht mehr Energie)",
        "ms(16~1000, 작을수록 반응이 빠르고 전력 소모가 큽니다)",
        "ms (16~1000; càng nhỏ càng mượt và càng tốn điện)",
        "ms (16~1000; küçük değer daha akıcı ve daha fazla güç tüketir)",
        "мс (16~1000; меньше — отзывчивее и энергозатратнее)",
        "ms (16~1000; mniej — płynniej i więcej zużywa energii)";

    // —— panels/settings.rs：发送 ——
    payload_format: "负载格式", "Payload format",
        "負載格式",
        "ペイロード形式",
        "Formato de carga",
        "Formato payload",
        "Formato de carga",
        "Format muatan",
        "Format de charge utile",
        "Nutzlastformat",
        "페이로드 형식",
        "Định dạng tải",
        "Yük biçimi",
        "Формат данных",
        "Format danych";
    hex_note:
        "HEX 时输入按十六进制解析：允许空格 / 逗号分隔与 0x 前缀；解析失败不会发送，也不会清空输入框",
        "In HEX mode the input is parsed as hexadecimal: spaces/commas and a 0x prefix are allowed; a parse failure sends nothing and does not clear the input",
        "HEX 時輸入按十六進位解析：允許空格 / 逗號分隔與 0x 前綴；解析失敗不會傳送，也不會清空輸入框",
        "HEX 時は入力を 16 進数として解釈します。スペース / カンマ区切りと 0x 接頭辞が使えます。解析に失敗した場合は送信されず、入力欄も消去されません",
        "En modo HEX la entrada se interpreta como hexadecimal: se permiten espacios/comas y el prefijo 0x; si falla el análisis no se envía nada ni se borra la entrada",
        "In modalità HEX l'input è interpretato come esadecimale: sono ammessi spazi/virgole e il prefisso 0x; se l'analisi fallisce non si invia nulla né si cancella l'input",
        "No modo HEX a entrada é interpretada como hexadecimal: espaços/vírgulas e o prefixo 0x são aceitos; se a análise falhar, nada é enviado nem a entrada é apagada",
        "Dalam mode HEX, masukan diurai sebagai heksadesimal: spasi/koma dan awalan 0x diizinkan; jika penguraian gagal, tidak ada yang dikirim dan kotak masukan tidak dibersihkan",
        "En mode HEX, l'entrée est interprétée en hexadécimal : espaces/virgules et préfixe 0x autorisés ; en cas d'échec d'analyse, rien n'est envoyé et l'entrée n'est pas effacée",
        "Im HEX-Modus wird die Eingabe als Hexadezimal interpretiert: Leerzeichen/Kommas und das Präfix 0x sind erlaubt; bei einem Analysefehler wird nichts gesendet und das Feld nicht geleert",
        "HEX 모드에서는 입력을 16진수로 해석합니다. 공백/쉼표 구분과 0x 접두사를 허용합니다. 해석에 실패하면 전송하지 않고 입력 상자도 비우지 않습니다",
        "Ở chế độ HEX, nội dung nhập được phân tích theo hệ thập lục phân: cho phép dấu cách / dấu phẩy và tiền tố 0x; nếu phân tích thất bại sẽ không gửi và không xóa ô nhập",
        "HEX modunda girdi onaltılık olarak ayrıştırılır: boşluk/virgül ayırıcı ve 0x öneki kabul edilir; ayrıştırma başarısız olursa hiçbir şey gönderilmez ve giriş temizlenmez",
        "В режиме HEX ввод разбирается как шестнадцатеричный: допускаются пробелы, запятые и префикс 0x; при ошибке разбора ничего не отправляется и поле не очищается",
        "W trybie HEX dane wejściowe są analizowane jako szesnastkowe: dozwolone są spacje, przecinki i prefiks 0x; przy błędzie analizy nic nie zostanie wysłane ani wyczyszczone";
    escape_note:
        "转义时 \\r \\n \\t \\0 \\xNN \\\\ 会变成对应的字节，其余文字仍按上面的编码发送；认不出的转义不会发送，也不会清空输入框",
        "In escape mode \\r \\n \\t \\0 \\xNN \\\\ become raw bytes and the remaining text is still encoded as above; an unrecognised escape sends nothing and does not clear the input",
        "跳脫時 \\r \\n \\t \\0 \\xNN \\\\ 會變成對應的位元組，其餘文字仍按上面的編碼傳送；認不出的跳脫不會傳送，也不會清空輸入框",
        "エスケープ時は \\r \\n \\t \\0 \\xNN \\\\ が対応するバイトになり、残りの文字は上記のエンコーディングで送信されます。認識できないエスケープは送信されず、入力欄も消去されません",
        "En modo escape, \\r \\n \\t \\0 \\xNN \\\\ se convierten en los bytes correspondientes y el texto restante se codifica como arriba; un escape no reconocido no envía nada ni borra la entrada",
        "In modalità escape, \\r \\n \\t \\0 \\xNN \\\\ diventano i byte corrispondenti e il resto del testo è codificato come sopra; un escape non riconosciuto non invia nulla né cancella l'input",
        "No modo escape, \\r \\n \\t \\0 \\xNN \\\\ viram os bytes correspondentes e o restante do texto é codificado como acima; um escape não reconhecido não envia nada nem apaga a entrada",
        "Dalam mode escape, \\r \\n \\t \\0 \\xNN \\\\ menjadi bita yang sesuai dan sisa teks tetap dikodekan seperti di atas; escape yang tidak dikenal tidak mengirim apa pun dan tidak membersihkan masukan",
        "En mode échappement, \\r \\n \\t \\0 \\xNN \\\\ deviennent les octets correspondants et le reste du texte est encodé comme ci-dessus ; un échappement non reconnu n'envoie rien et n'efface pas l'entrée",
        "Im Escape-Modus werden \\r \\n \\t \\0 \\xNN \\\\ zu den entsprechenden Bytes und der restliche Text wird wie oben kodiert; ein unbekanntes Escape sendet nichts und leert das Feld nicht",
        "이스케이프 모드에서는 \\r \\n \\t \\0 \\xNN \\\\ 이 해당 바이트가 되고 나머지 텍스트는 위의 인코딩으로 전송됩니다. 인식할 수 없는 이스케이프는 전송하지 않고 입력 상자도 비우지 않습니다",
        "Ở chế độ thoát, \\r \\n \\t \\0 \\xNN \\\\ trở thành các byte tương ứng và phần văn bản còn lại vẫn được mã hóa như trên; ký tự thoát không nhận diện được sẽ không gửi và không xóa ô nhập",
        "Kaçış modunda \\r \\n \\t \\0 \\xNN \\\\ karşılık gelen baytlara dönüşür ve kalan metin yine yukarıdaki kodlamayla gönderilir; tanınmayan bir kaçış hiçbir şey göndermez ve girişi temizlemez",
        "В режиме экранирования \\r \\n \\t \\0 \\xNN \\\\ превращаются в соответствующие байты, а остальной текст по-прежнему кодируется как выше; нераспознанная последовательность ничего не отправляет и не очищает поле",
        "W trybie znaków ucieczki \\r \\n \\t \\0 \\xNN \\\\ stają się odpowiednimi bajtami, a pozostały tekst jest nadal kodowany jak wyżej; nierozpoznana sekwencja nic nie wysyła i nie czyści pola";
    ending_note:
        "追加在字节层：HEX 模式下发出的是 0x0A / 0x0D 0x0A 两个字节，不是字符",
        "Appended at the byte level: in HEX mode this emits the bytes 0x0A / 0x0D 0x0A, not characters",
        "附加在位元組層：HEX 模式下發出的是 0x0A / 0x0D 0x0A 兩個位元組，不是字元",
        "バイト層で付加されます。HEX モードでは 0x0A / 0x0D 0x0A のバイトが送られ、文字ではありません",
        "Se añade a nivel de byte: en modo HEX emite los bytes 0x0A / 0x0D 0x0A, no caracteres",
        "Aggiunto a livello di byte: in modalità HEX emette i byte 0x0A / 0x0D 0x0A, non caratteri",
        "Anexado no nível de byte: no modo HEX emite os bytes 0x0A / 0x0D 0x0A, não caracteres",
        "Ditambahkan di tingkat bita: dalam mode HEX mengirim bita 0x0A / 0x0D 0x0A, bukan karakter",
        "Ajouté au niveau des octets : en mode HEX, émet les octets 0x0A / 0x0D 0x0A, pas des caractères",
        "Auf Byte-Ebene angehängt: im HEX-Modus werden die Bytes 0x0A / 0x0D 0x0A gesendet, keine Zeichen",
        "바이트 계층에서 추가됩니다. HEX 모드에서는 0x0A / 0x0D 0x0A 바이트가 전송되며 문자는 아닙니다",
        "Được thêm ở tầng byte: ở chế độ HEX sẽ gửi các byte 0x0A / 0x0D 0x0A, không phải ký tự",
        "Bayt düzeyinde eklenir: HEX modunda 0x0A / 0x0D 0x0A baytları gönderilir, karakter değil",
        "Добавляется на уровне байтов: в режиме HEX отправляются байты 0x0A / 0x0D 0x0A, а не символы",
        "Dodawane na poziomie bajtów: w trybie HEX wysyłane są bajty 0x0A / 0x0D 0x0A, a nie znaki";
    clear_after_send: "发送后清空输入框", "Clear input after sending",
        "傳送後清空輸入框",
        "送信後に入力欄を消去",
        "Borrar la entrada tras enviar",
        "Cancella l'input dopo l'invio",
        "Limpar a entrada após enviar",
        "Bersihkan masukan setelah mengirim",
        "Effacer l'entrée après envoi",
        "Eingabe nach dem Senden löschen",
        "전송 후 입력 상자 비우기",
        "Xóa ô nhập sau khi gửi",
        "Gönderdikten sonra girişi temizle",
        "Очищать поле после отправки",
        "Wyczyść pole po wysłaniu";
    repeat_interval_field: "定时发送间隔", "Repeat interval",
        "定時傳送間隔",
        "定期送信の間隔",
        "Intervalo de envío periódico",
        "Intervallo di invio periodico",
        "Intervalo de envio periódico",
        "Interval pengiriman berkala",
        "Intervalle d'envoi périodique",
        "Intervall für wiederholtes Senden",
        "반복 전송 간격",
        "Khoảng gửi định kỳ",
        "Yineli gönderme aralığı",
        "Интервал повторной отправки",
        "Interwał wysyłania cyklicznego";
    repeat_interval_range: "ms（{}~{}）", "ms ({}~{})",
        "ms（{}~{}）",
        "ms（{}~{}）",
        "ms ({}~{})",
        "ms ({}~{})",
        "ms ({}~{})",
        "ms ({}~{})",
        "ms ({}~{})",
        "ms ({}~{})",
        "ms ({}~{})",
        "ms ({}~{})",
        "ms ({}~{})",
        "мс ({}~{})",
        "ms ({}~{})";
    repeat_note:
        "开关在发送区；断开连接会自动关掉，且不跨进程延续（重启后一定是关的）",
        "The switch lives in the send area; disconnecting turns it off automatically, and it never survives a restart",
        "開關在傳送區；中斷連線會自動關掉，且不跨行程延續（重啟後一定是關的）",
        "スイッチは送信エリアにあります。切断すると自動でオフになり、プロセスをまたいで維持されません（再起動後は必ずオフ）",
        "El interruptor está en el área de envío; al desconectar se apaga automáticamente y no se conserva entre procesos (tras reiniciar siempre está apagado)",
        "L'interruttore è nell'area di invio; disconnettendosi si spegne automaticamente e non persiste tra processi (dopo il riavvio è sempre spento)",
        "O interruptor fica na área de envio; ao desconectar, desliga automaticamente e não persiste entre processos (após reiniciar está sempre desligado)",
        "Sakelar ada di area kirim; memutuskan koneksi akan mematikannya otomatis, dan tidak bertahan antar proses (setelah mulai ulang selalu mati)",
        "L'interrupteur se trouve dans la zone d'envoi ; à la déconnexion, il s'éteint automatiquement et ne survit pas au redémarrage (toujours éteint après redémarrage)",
        "Der Schalter liegt im Sendebereich; beim Trennen schaltet er sich automatisch aus und überlebt keinen Neustart (nach dem Neustart immer aus)",
        "스위치는 송신 영역에 있습니다. 연결을 끊으면 자동으로 꺼지며 프로세스를 넘어 유지되지 않습니다(재시작 후에는 항상 꺼짐)",
        "Công tắc nằm ở vùng gửi; khi ngắt kết nối sẽ tự tắt và không giữ qua lần chạy sau (sau khi khởi động lại luôn tắt)",
        "Anahtar gönderme alanındadır; bağlantı kesilince kendiliğinden kapanır ve yeniden başlatmayı aşmaz (yeniden başlatınca her zaman kapalıdır)",
        "Переключатель находится в области отправки; при отключении выключается сам и не переживает перезапуск (после перезапуска всегда выключен)",
        "Przełącznik znajduje się w obszarze wysyłania; przy rozłączeniu wyłącza się sam i nie przetrwa restartu (po restarcie zawsze wyłączony)";
    send_history_count_text: "{} / {} 条", "{} / {} entries",
        "{} / {} 筆",
        "{} / {} 件",
        "{} / {} entradas",
        "{} / {} voci",
        "{} / {} entradas",
        "{} / {} entri",
        "{} / {} entrées",
        "{} / {} Einträge",
        "{} / {} 개",
        "{} / {} mục",
        "{} / {} kayıt",
        "{} / {} записей",
        "{} / {} pozycji";
    history_note:
        "↑ / ↓ 在输入框首行 / 末行时取历史（首行按 ↑、末行按 ↓）；连续发送相同内容只记一条",
        "↑ / ↓ recall history at the first/last line of the input (↑ on the first line, ↓ on the last); repeated identical sends are stored once",
        "↑ / ↓ 在輸入框首行 / 末行時取歷史（首行按 ↑、末行按 ↓）；連續傳送相同內容只記一條",
        "↑ / ↓ は入力欄の先頭行 / 最終行で履歴を呼び出します（先頭行で ↑、最終行で ↓）。同じ内容を連続送信しても 1 件だけ記録します",
        "↑ / ↓ recuperan el historial en la primera/última línea de la entrada (↑ en la primera, ↓ en la última); los envíos idénticos repetidos se guardan una sola vez",
        "↑ / ↓ richiamano la cronologia sulla prima/ultima riga dell'input (↑ sulla prima, ↓ sull'ultima); invii identici ripetuti vengono memorizzati una sola volta",
        "↑ / ↓ recuperam o histórico na primeira/última linha da entrada (↑ na primeira, ↓ na última); envios idênticos repetidos são guardados uma só vez",
        "↑ / ↓ memanggil riwayat di baris pertama/terakhir masukan (↑ di baris pertama, ↓ di baris terakhir); pengiriman identik yang berulang hanya disimpan sekali",
        "↑ / ↓ rappellent l'historique à la première/dernière ligne de l'entrée (↑ sur la première, ↓ sur la dernière) ; les envois identiques répétés ne sont enregistrés qu'une fois",
        "↑ / ↓ rufen den Verlauf in der ersten/letzten Zeile des Eingabefelds ab (↑ in der ersten, ↓ in der letzten); wiederholte identische Sendungen werden nur einmal gespeichert",
        "↑ / ↓ 는 입력 상자의 첫 줄/마지막 줄에서 기록을 불러옵니다(첫 줄에서 ↑, 마지막 줄에서 ↓). 같은 내용을 연속 전송해도 한 번만 기록합니다",
        "↑ / ↓ gọi lịch sử ở dòng đầu/dòng cuối của ô nhập (↑ ở dòng đầu, ↓ ở dòng cuối); gửi liên tiếp cùng nội dung chỉ lưu một mục",
        "↑ / ↓, girişin ilk/son satırındayken geçmişi çağırır (ilk satırda ↑, son satırda ↓); art arda aynı içeriği göndermek tek kayıt oluşturur",
        "↑ / ↓ вызывают историю в первой/последней строке поля ввода (↑ в первой, ↓ в последней); повторная отправка одинакового текста сохраняется один раз",
        "↑ / ↓ przywołują historię w pierwszym/ostatnim wierszu pola (↑ w pierwszym, ↓ w ostatnim); powtórne wysłanie tej samej treści zapisuje się raz";

    // —— panels/settings.rs：实时图表 ——
    chart_panel_checkbox:
        "显示图表面板（也可从「视图 → 面板」切换）",
        "Show the chart panel (also toggled from View → Panels)",
        "顯示圖表面板（也可從「檢視 → 面板」切換）",
        "チャートパネルを表示（「表示 → パネル」からも切り替え可）",
        "Mostrar el panel del gráfico (también se alterna en Ver → Paneles)",
        "Mostra il pannello del grafico (si attiva anche da Visualizza → Pannelli)",
        "Mostrar o painel do gráfico (também alternado em Exibir → Painéis)",
        "Tampilkan panel grafik (juga diubah dari Tampilan → Panel)",
        "Afficher le panneau du graphique (aussi via Affichage → Panneaux)",
        "Diagrammbereich anzeigen (auch über Ansicht → Bereiche umschaltbar)",
        "차트 패널 표시(보기 → 패널에서도 전환 가능)",
        "Hiện bảng biểu đồ (cũng chuyển được từ Xem → Bảng điều khiển)",
        "Grafik panelini göster (Görünüm → Paneller'den de değiştirilebilir)",
        "Показывать панель графика (также переключается в «Вид → Панели»)",
        "Pokaż panel wykresu (przełączysz też w Widok → Panele)";
    max_samples_field: "样本上限", "Max samples",
        "樣本上限",
        "サンプル上限",
        "Máximo de muestras",
        "Massimo campioni",
        "Máximo de amostras",
        "Batas sampel",
        "Maximum d'échantillons",
        "Max. Messwerte",
        "샘플 상한",
        "Giới hạn mẫu",
        "Örnek üst sınırı",
        "Предел значений",
        "Limit próbek";
    samples_range: "点/曲线（{}~{}）", "points/curve ({}~{})",
        "點/曲線（{}~{}）",
        "点/曲線（{}~{}）",
        "puntos/curva ({}~{})",
        "punti/curva ({}~{})",
        "pontos/curva ({}~{})",
        "titik/kurva ({}~{})",
        "points/courbe ({}~{})",
        "Punkte/Kurve ({}~{})",
        "점/곡선 ({}~{})",
        "điểm/đường ({}~{})",
        "nokta/eğri ({}~{})",
        "точек/кривая ({}~{})",
        "punktów/krzywa ({}~{})";
    samples_note:
        "超出上限时从最旧的样本开始淘汰；绘制时每条曲线最多取 4000 点",
        "When over the limit the oldest samples are dropped; each curve is drawn with at most 4000 points",
        "超出上限時從最舊的樣本開始淘汰；繪製時每條曲線最多取 4000 點",
        "上限を超えると古いサンプルから破棄されます。描画時、各曲線は最大 4000 点を使用します",
        "Al superar el límite se descartan las muestras más antiguas; cada curva se dibuja con 4000 puntos como máximo",
        "Superato il limite, si scartano i campioni più vecchi; ogni curva è disegnata con al massimo 4000 punti",
        "Ao ultrapassar o limite, as amostras mais antigas são descartadas; cada curva é desenhada com no máximo 4000 pontos",
        "Jika melebihi batas, sampel terlama dibuang; setiap kurva digambar dengan maksimal 4000 titik",
        "Au-delà de la limite, les échantillons les plus anciens sont supprimés ; chaque courbe est tracée avec au plus 4000 points",
        "Über dem Limit werden die ältesten Messwerte verworfen; jede Kurve wird mit höchstens 4000 Punkten gezeichnet",
        "상한을 넘으면 가장 오래된 샘플부터 버립니다. 그릴 때 각 곡선은 최대 4000개 점을 사용합니다",
        "Khi vượt giới hạn, mẫu cũ nhất bị loại bỏ; mỗi đường cong được vẽ với tối đa 4000 điểm",
        "Üst sınır aşıldığında en eski örnekler atılır; çizerken her eğri en fazla 4000 nokta kullanır",
        "При превышении предела отбрасываются самые старые значения; при отрисовке каждая кривая использует не более 4000 точек",
        "Po przekroczeniu limitu odrzucane są najstarsze próbki; przy rysowaniu każda krzywa używa maksymalnie 4000 punktów";
    auto_bounds_checkbox:
        "自动缩放（关掉后保留手动拖动/缩放的视图）",
        "Auto bounds (when off, your manual pan/zoom is kept)",
        "自動縮放（關掉後保留手動拖曳/縮放的檢視）",
        "自動スケール（オフにすると手動のパン/ズーム表示を保持）",
        "Escala automática (si se desactiva, se conserva el encuadre manual)",
        "Scala automatica (se disattivata, mantiene la vista manuale)",
        "Escala automática (se desativada, mantém a vista manual)",
        "Skala otomatis (jika dimatikan, tampilan geser/zoom manual dipertahankan)",
        "Échelle automatique (si désactivée, le cadrage manuel est conservé)",
        "Automatischer Maßstab (wenn aus, bleibt die manuelle Ansicht erhalten)",
        "자동 크기 조정(끄면 수동 이동/확대 보기가 유지됨)",
        "Tự động điều chỉnh (nếu tắt, giữ nguyên chế độ kéo/thu phóng thủ công)",
        "Otomatik sınırlar (kapatınca elle kaydırma/yakınlaştırma korunur)",
        "Автомасштаб (при выключении сохраняется ручной сдвиг/масштаб)",
        "Automatyczne granice (po wyłączeniu zachowywane jest ręczne przesuwanie/zbliżenie)";
    current_data: "当前数据", "Current data",
        "目前資料",
        "現在のデータ",
        "Datos actuales",
        "Dati attuali",
        "Dados atuais",
        "Data saat ini",
        "Données actuelles",
        "Aktuelle Daten",
        "현재 데이터",
        "Dữ liệu hiện tại",
        "Geçerli veriler",
        "Текущие данные",
        "Bieżące dane";
    chart_count_text: "{} 通道 · {} 样本", "{} channels · {} samples",
        "{} 通道 · {} 樣本",
        "{} チャネル · {} サンプル",
        "{} canales · {} muestras",
        "{} canali · {} campioni",
        "{} canais · {} amostras",
        "{} kanal · {} sampel",
        "{} canaux · {} échantillons",
        "{} Kanäle · {} Messwerte",
        "{} 채널 · {} 샘플",
        "{} kênh · {} mẫu",
        "{} kanal · {} örnek",
        "{} каналов · {} значений",
        "{} kanałów · {} próbek";
    chart_format_note:
        "格式：一行一组样本，字段用逗号分隔（如 1.0,2.0,3.0）；与接收显示模式无关",
        "Format: one sample group per line, comma-separated fields (e.g. 1.0,2.0,3.0); independent of the receive display mode",
        "格式：一行一組樣本，欄位用逗號分隔（如 1.0,2.0,3.0）；與接收顯示模式無關",
        "形式: 1 行 1 組のサンプル、フィールドはカンマ区切り（例 1.0,2.0,3.0）。受信の表示モードとは無関係です",
        "Formato: un grupo de muestras por línea, campos separados por comas (p. ej. 1.0,2.0,3.0); independiente del modo de vista de recepción",
        "Formato: un gruppo di campioni per riga, campi separati da virgole (es. 1.0,2.0,3.0); indipendente dalla modalità di vista della ricezione",
        "Formato: um grupo de amostras por linha, campos separados por vírgulas (ex. 1.0,2.0,3.0); independente do modo de exibição da recepção",
        "Format: satu grup sampel per baris, bidang dipisah koma (mis. 1.0,2.0,3.0); tidak bergantung pada mode tampilan penerimaan",
        "Format : un groupe d'échantillons par ligne, champs séparés par des virgules (ex. 1.0,2.0,3.0) ; indépendant du mode d'affichage de la réception",
        "Format: eine Messwertgruppe pro Zeile, Felder durch Kommas getrennt (z. B. 1.0,2.0,3.0); unabhängig vom Anzeigemodus des Empfangs",
        "형식: 한 줄에 샘플 한 그룹, 필드는 쉼표로 구분(예: 1.0,2.0,3.0). 수신 표시 모드와는 무관합니다",
        "Định dạng: một nhóm mẫu mỗi dòng, các trường cách nhau bằng dấu phẩy (ví dụ 1.0,2.0,3.0); không phụ thuộc chế độ hiển thị khi nhận",
        "Biçim: satır başına bir örnek grubu, alanlar virgülle ayrılır (örn. 1.0,2.0,3.0); alım görüntüleme modundan bağımsızdır",
        "Формат: одна группа значений в строке, поля через запятую (например 1.0,2.0,3.0); не зависит от режима отображения приёма",
        "Format: jedna grupa próbek w wierszu, pola oddzielone przecinkami (np. 1.0,2.0,3.0); niezależny od trybu wyświetlania odbioru";

    // —— panels/settings.rs：文件日志 ——
    file_name_template: "文件名模板", "File name template",
        "檔案名稱範本",
        "ファイル名テンプレート",
        "Plantilla de nombre de archivo",
        "Modello nome file",
        "Modelo de nome de arquivo",
        "Templat nama berkas",
        "Modèle de nom de fichier",
        "Dateinamensvorlage",
        "파일 이름 템플릿",
        "Mẫu tên tệp",
        "Dosya adı şablonu",
        "Шаблон имени файла",
        "Szablon nazwy pliku";
    next_file_name:
        "下次开始记录时生成: {}",
        "Will be created when logging next starts: {}",
        "下次開始記錄時產生: {}",
        "次回記録開始時に生成: {}",
        "Se creará al iniciar el registro: {}",
        "Sarà creato al prossimo avvio della registrazione: {}",
        "Será criado ao iniciar o registro: {}",
        "Akan dibuat saat pencatatan berikutnya dimulai: {}",
        "Sera créé au prochain démarrage de la journalisation : {}",
        "Wird beim nächsten Aufzeichnungsstart erstellt: {}",
        "다음 기록 시작 시 생성됩니다: {}",
        "Sẽ được tạo khi bắt đầu ghi lần sau: {}",
        "Kayıt bir sonraki başlatıldığında oluşturulur: {}",
        "Будет создан при следующем запуске записи: {}",
        "Zostanie utworzony przy następnym uruchomieniu zapisu: {}";
    output_dir_field: "输出目录", "Output directory",
        "輸出目錄",
        "出力先",
        "Carpeta de salida",
        "Cartella di output",
        "Pasta de saída",
        "Direktori keluaran",
        "Dossier de sortie",
        "Ausgabeordner",
        "출력 디렉터리",
        "Thư mục đầu ra",
        "Çıktı klasörü",
        "Папка вывода",
        "Katalog wyjściowy";
    restore_default: "恢复默认", "Restore default",
        "恢復預設",
        "既定に戻す",
        "Restaurar predeterminado",
        "Ripristina predefinito",
        "Restaurar padrão",
        "Pulihkan bawaan",
        "Rétablir par défaut",
        "Standard wiederherstellen",
        "기본값 복원",
        "Khôi phục mặc định",
        "Varsayılana dön",
        "Восстановить по умолчанию",
        "Przywróć domyślne";
    logging_written: "记录中，已写 {} 字节", "Logging, {} bytes written",
        "記錄中，已寫 {} 位元組",
        "記録中、{} バイト書き込み済み",
        "Registrando, {} bytes escritos",
        "Registrazione in corso, {} byte scritti",
        "Registrando, {} bytes gravados",
        "Mencatat, {} bita tertulis",
        "Journalisation, {} octets écrits",
        "Aufzeichnung läuft, {} Bytes geschrieben",
        "기록 중, {} 바이트 작성됨",
        "Đang ghi, đã viết {} byte",
        "Kaydediliyor, {} bayt yazıldı",
        "Запись идёт, записано {} байт",
        "Zapisywanie, zapisano {} bajtów";
    log_note:
        "接收区的内容会同步写入该文件；日志文件不受上面的长度上限约束",
        "Whatever appears in the receive area is written to this file; the file is not subject to the length limit above",
        "接收區的內容會同步寫入該檔案；日誌檔案不受上面的長度上限約束",
        "受信エリアの内容はこのファイルに同期して書き込まれます。ログファイルは上記の長さ上限の対象外です",
        "El contenido del área de recepción se escribe en este archivo; el archivo de registro no está sujeto al límite de longitud anterior",
        "Il contenuto dell'area di ricezione viene scritto in questo file; il file di log non è soggetto al limite di lunghezza sopra",
        "O conteúdo da área de recepção é gravado neste arquivo; o arquivo de log não está sujeito ao limite de comprimento acima",
        "Isi area penerimaan ditulis ke berkas ini; berkas log tidak tunduk pada batas panjang di atas",
        "Le contenu de la zone de réception est écrit dans ce fichier ; le fichier journal n'est pas soumis à la limite de longueur ci-dessus",
        "Der Inhalt des Empfangsbereichs wird in diese Datei geschrieben; die Protokolldatei unterliegt nicht der obigen Längenbegrenzung",
        "수신 영역의 내용이 이 파일에 함께 기록됩니다. 로그 파일은 위의 길이 상한에 제약받지 않습니다",
        "Nội dung vùng nhận được ghi vào tệp này; tệp log không bị ràng buộc bởi giới hạn độ dài ở trên",
        "Alım alanındaki içerik bu dosyaya eşzamanlı yazılır; günlük dosyası yukarıdaki uzunluk sınırına tabi değildir",
        "Содержимое области приёма синхронно записывается в этот файл; на файл журнала предел длины выше не распространяется",
        "Treść obszaru odbioru jest zapisywana na bieżąco w tym pliku; plik dziennika nie podlega powyższemu limitowi długości";
    prefs_note:
        "以上均为用户偏好，改动即时生效并在退出时自动保存",
        "Everything above is a user preference: changes apply immediately and are saved on exit",
        "以上均為使用者偏好，改動即時生效並在結束時自動儲存",
        "以上はすべてユーザー設定です。変更はすぐ反映され、終了時に自動保存されます",
        "Todo lo anterior son preferencias del usuario: los cambios se aplican al momento y se guardan al salir",
        "Tutto quanto sopra sono preferenze utente: le modifiche hanno effetto immediato e vengono salvate all'uscita",
        "Tudo acima são preferências do usuário: as mudanças entram em vigor na hora e são salvas ao sair",
        "Semua di atas adalah preferensi pengguna: perubahan langsung berlaku dan disimpan saat keluar",
        "Tout ce qui précède relève des préférences utilisateur : les modifications prennent effet immédiatement et sont enregistrées à la fermeture",
        "Alles oben sind Nutzereinstellungen: Änderungen wirken sofort und werden beim Beenden gespeichert",
        "위 항목은 모두 사용자 환경설정입니다. 변경은 즉시 적용되며 종료 시 자동 저장됩니다",
        "Tất cả nội dung trên là tùy chọn người dùng: thay đổi có hiệu lực ngay và được tự động lưu khi thoát",
        "Yukarıdakilerin tümü kullanıcı tercihleridir: değişiklikler anında geçerli olur ve çıkışta otomatik kaydedilir",
        "Всё выше — пользовательские предпочтения: изменения применяются сразу и сохраняются при выходе",
        "Wszystko powyżej to preferencje użytkownika: zmiany działają natychmiast i są zapisywane przy wyjściu";

    // —— receive.rs：接收区右键菜单 ——
    copy_all_log: "复制全部日志", "Copy the whole log",
        "複製全部日誌",
        "ログをすべてコピー",
        "Copiar todo el registro",
        "Copia tutto il log",
        "Copiar todo o log",
        "Salin seluruh log",
        "Copier tout le journal",
        "Gesamtes Protokoll kopieren",
        "전체 로그 복사",
        "Sao chép toàn bộ log",
        "Tüm günlüğü kopyala",
        "Копировать весь журнал",
        "Kopiuj cały dziennik";
}

impl Language {
    /// 该语言对应的文案表。
    ///
    /// 返回 `&'static`：调用处拿到的引用不借用 `self`，因此可以在
    /// `&mut self` 的方法体里随时取用，不会与其它字段的可变借用打架。
    ///
    /// 这个 `match` 的**臂顺序必须与 [`Language::ALL`] 及表里 `langs:` 的顺序一致**
    /// ——`tests::every_language_variant_maps_to_a_table` 与
    /// `tests::langs_table_matches_language_order` 一起把三处钉死。
    pub(crate) fn strings(self) -> &'static Strings {
        match self {
            Self::Zh => &ZH,
            Self::En => &EN,
            Self::ZhHant => &ZH_HANT,
            Self::Ja => &JA,
            Self::Es => &ES,
            Self::It => &IT,
            Self::Pt => &PT,
            Self::Id => &ID,
            Self::Fr => &FR,
            Self::De => &DE,
            Self::Ko => &KO,
            Self::Vi => &VI,
            Self::Tr => &TR,
            Self::Ru => &RU,
            Self::Pl => &PL,
        }
    }
}

/// `TimestampPrefix` 的界面标签。
///
/// 放在文案表这边而不是 `receive.rs` 里：这三个词是**界面词汇**，不是领域概念。
/// （旧版 `TimestampPrefix::label()` 硬编码中文，已随 F3 删除——留着它迟早有人
/// 在别处顺手用上，于是漏翻译。）
pub(crate) fn timestamp_prefix_label(strings: &Strings, prefix: TimestampPrefix) -> &'static str {
    match prefix {
        TimestampPrefix::Off => strings.ts_off,
        TimestampPrefix::PerLine => strings.ts_per_line,
        TimestampPrefix::PerBatch => strings.ts_per_batch,
    }
}

/// 把模板里的 `{}` 依次替换成实参。
///
/// 存在的唯一原因：`format!` 的格式串必须是字面量，而这里的格式串来自文案表
/// （运行期才决定是中文还是英文）。功能上等价于 `format!(模板, 参数…)`。
///
/// 实参多于 `{}` 时多余的被忽略；少于 `{}` 时**保留**未填充的 `{}`（不 panic）。
/// 两种情形在界面上都不该出现，`tests::every_template_placeholder_is_filled`
/// 用真实的调用参数数把这些模板钉住。
pub(crate) fn fill(template: &str, args: &[&dyn std::fmt::Display]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    let mut next = 0;

    while let Some(pos) = rest.find("{}") {
        out.push_str(&rest[..pos]);
        if let Some(arg) = args.get(next) {
            use std::fmt::Write as _;
            // 写进 String 不会失败（`fmt::Write for String` 永远返回 Ok）
            let _ = write!(out, "{arg}");
        } else {
            out.push_str("{}");
        }
        next += 1;
        rest = &rest[pos + 2..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 各语言文字所属的**文字系统**（script）判定。
    ///
    /// 加语言后，早先那条"英文列不许出现 CJK"的单一规则不够用了：日语列出现汉字
    /// 是**对的**，拉丁语系列出现 CJK 才是漏翻译。所以把判据从"一列一个规则"改成
    /// "**每种语言列出它不该出现的文字系统**"，逐语言校验（见
    /// [`columns_have_no_foreign_script`]）。
    ///
    /// 只做**互斥**的粗粒度判定，不追求 Unicode 完整性：够抓住"整列粘错/漏翻"就够，
    /// 精确的字体覆盖由 `main.rs` 的 `setup_fonts` 注释 + 越南语专项测试兜。
    fn has_cjk(text: &str) -> bool {
        text.chars().any(|c| {
            matches!(c,
                '\u{2E80}'..='\u{303F}'   // 部首扩展 + 中日韩符号与标点（。、「」等）
                | '\u{3400}'..='\u{4DBF}' // 扩展 A
                | '\u{4E00}'..='\u{9FFF}' // 基本区
                | '\u{F900}'..='\u{FAFF}' // 兼容表意文字
                | '\u{FF00}'..='\u{FFEF}' // 全角形式（（）等）
            )
        })
    }

    /// 平假名（U+3040–309F）与片假名（U+30A0–30FF）。
    fn has_kana(text: &str) -> bool {
        text.chars()
            .any(|c| matches!(c, '\u{3040}'..='\u{309F}' | '\u{30A0}'..='\u{30FF}'))
    }

    /// 谚文（Hangul）：音节（AC00–D7AF）、字母（1100–11FF）、兼容字母（3130–318F）。
    fn has_hangul(text: &str) -> bool {
        text.chars().any(|c| {
            matches!(c, '\u{AC00}'..='\u{D7AF}' | '\u{1100}'..='\u{11FF}' | '\u{3130}'..='\u{318F}')
        })
    }

    /// 泰文（U+0E00–U+0E7F）。
    fn has_thai(text: &str) -> bool {
        text.chars().any(|c| matches!(c, '\u{0E00}'..='\u{0E7F}'))
    }

    /// 西里尔字母：基本区（U+0400–U+04FF）与补充区（U+0500–U+052F）。
    ///
    /// 第三批加俄语后**必须**有这个检测器：没有它，把俄语列整列换成波兰语
    /// （纯拉丁）也不会被判红——两列都是"非 CJK"，粗看都正常。
    /// 这正是本轮实测到的空缺（见 [`script_allowed`] 与本函数使用处的注释）。
    fn has_cyrillic(text: &str) -> bool {
        text.chars()
            .any(|c| matches!(c, '\u{0400}'..='\u{04FF}' | '\u{0500}'..='\u{052F}'))
    }

    /// 拉丁字母含变音符号（越南语、以及西/葡/法/德/意等都需要）。
    fn has_latin_ext(text: &str) -> bool {
        text.chars().any(|c| {
            matches!(c, '\u{00C0}'..='\u{024F}' // 拉丁字母 Supplement + Extended-A/B
                      | '\u{1E00}'..='\u{1EFF}') // 拉丁字母 Extended Additional（越南语重灾区）
        })
    }

    /// 每种语言**允许**出现的文字系统。列出的之外的系统出现在该列 = 漏翻译/粘错列。
    ///
    /// `true` = 该语言**可以**含这种文字。混合文字的语言（日语 = 假名 + 汉字；
    /// 韩语 = 谚文 + 少量汉字）在这里显式放开，不必靠"恰好没被扫到"过关。
    fn script_allowed(language: Language, script: &str) -> bool {
        use Language::*;
        match (language, script) {
            (_, "kana") => matches!(language, Ja),
            (_, "hangul") => matches!(language, Ko),
            // 泰语尚未纳入（第三批的下一档，见 I18N_REMAINING_LANGUAGES.md §5.1）；
            // 届时在此放行 `Th`，并先跑叠置记号的实机探针。
            (_, "thai") => false,
            // 西里尔：只有俄语允许。第三批加 `ru` 后补上——**这是本轮实测出的空缺**：
            // 探针把 ru 列写成波兰语（拉丁）时，旧检测器集合（kana/hangul/thai/cjk）
            // 全部不命中，守卫照样绿。注意本项只挡"别列混入西里尔"，
            // "ru 列没有西里尔" 由 `third_batch_columns_use_their_own_alphabets` 反向兜。
            (_, "cyrillic") => matches!(language, Ru),
            // 汉字：中文（简/繁）与日语允许；韩语技术文案里几乎不用汉字，这里也放开
            // （谚文 + 汉字混排是韩语正当写法，不该被这条守卫判成漏翻译）。
            (_, "cjk") => matches!(language, Zh | ZhHant | Ja | Ko),
            _ => false,
        }
    }

    /// 各语言列里**不许**出现其它语言的专属文字系统。
    ///
    /// 这条取代了早先的 `english_column_has_no_cjk`：早先只有中英两列，一条
    /// "英文列不许有 CJK"就够；现在 12 列，必须按语言分别判定——否则日语列的
    /// 汉字会被误报，而日语列里**冒出谚文**这种真问题又不会被发现。
    #[test]
    fn columns_have_no_foreign_script() {
        /// (文字系统名, 判定函数)。抽成别名，免得 clippy 报 type_complexity。
        type ScriptCheck = (&'static str, fn(&str) -> bool);

        let checks: &[ScriptCheck] = &[
            ("kana", has_kana as fn(&str) -> bool),
            ("hangul", has_hangul),
            ("thai", has_thai),
            ("cyrillic", has_cyrillic),
            ("cjk", has_cjk),
        ];

        for (name, texts) in ALL {
            for (index, language) in Language::ALL.iter().enumerate() {
                let Some(text) = texts.get(index) else {
                    panic!("{name} 缺少 {language:?} 这一列（列数与 Language::ALL 不一致）");
                };
                for (script, detector) in checks {
                    if detector(text) && !script_allowed(*language, script) {
                        panic!(
                            "「{name}」的 {language:?} 列里出现了 {script} 字符，\
                             像是漏翻译或粘错了列：{text:?}"
                        );
                    }
                }
            }
        }
    }

    /// **越南语列**的字符必须真的用上了拉丁扩展区，否则"越南语"只是 ASCII 拉丁。
    ///
    /// 越南语字母表靠大量**预组合变音字符**（U+1EA0–U+1EF9：ạ ả ấ ầ ẩ ẫ ậ …）与
    /// 组合区（U+00C0–U+024F）。如果某列只写了纯 ASCII（把 `Tiếng Việt` 写成
    /// `Tieng Viet`），它"看起来像越南语"但实际是退化写法——这条把它钉死。
    ///
    /// 反过来，这条也保证 `has_latin_ext` 这个检测器**真的被测到**（不是死代码）。
    #[test]
    fn vietnamese_column_uses_latin_extended() {
        let index = Language::ALL
            .iter()
            .position(|l| *l == Language::Vi)
            .expect("Language::ALL 里必须有 Vi");

        let mut with_ext = 0usize;
        let mut total_chars = 0usize;
        for (_, texts) in ALL {
            let text = texts[index];
            total_chars += text.chars().count();
            if has_latin_ext(text) {
                with_ext += 1;
            }
        }
        assert!(
            with_ext >= 20,
            "越南语列只有 {with_ext} 条用到了拉丁扩展区（共 {total_chars} 字符）——\
             像是把变音符号丢了（`Tieng Viet` 而不是 `Tiếng Việt`）"
        );
    }

    /// 越南语用到的**每一个**字符，字体链里都必须有字形。
    ///
    /// 这是 I18N_TARGET_LANGUAGES.md §5 的要求：不能只抽查几个样本字符，要遍历
    /// 越南语列**实际出现的全部字符集**。抽查找不出"某个生僻变音字符恰好缺字形"
    /// 这类问题——它会静默显示成豆腐块 □。
    ///
    /// 字体链必须与 `main.rs::setup_fonts` **逐项一致**：微软雅黑（中文）→ tahoma
    /// （拉丁扩展）→ malgun（谚文）→ seguisym（符号）。雅黑缺全部 U+1EA0–U+1EF9，
    /// 靠 tahoma 兜底；tahoma 若读不到（非 Windows CI），本测试**跳过**而不是误报。
    ///
    /// ⚠️ **2026-09-27 修**：这份清单原先只列了雅黑与 tahoma，漏掉 malgun 与
    /// seguisym。字形检测器重写（见 [`font_has_glyph`]）之后这条测试立刻红在
    /// `☰`（`sidebar_toggle` 那个图标）上——它确实不在雅黑/tahoma 里，而在
    /// **seguisym** 里。漏列一个字体就是**假红**：测试在报一个用户根本不会遇到的
    /// 缺字（界面照常显示）。清单必须跟着真实字体链走。
    #[test]
    fn every_vietnamese_character_has_a_glyph_in_the_font_chain() {
        let index = Language::ALL
            .iter()
            .position(|l| *l == Language::Vi)
            .expect("Language::ALL 里必须有 Vi");

        // 收集越南语列里出现的**全部不同字符**。
        let mut charset: std::collections::BTreeSet<char> = std::collections::BTreeSet::new();
        for (_, texts) in ALL {
            charset.extend(texts[index].chars());
        }

        // 字体链：只要**任一**字体有该字符的字形即可。
        let candidates = [
            r"C:\Windows\Fonts\msyh.ttc",
            r"C:\Windows\Fonts\msyhbd.ttc",
            r"C:\Windows\Fonts\simsun.ttc",
            r"C:\Windows\Fonts\simhei.ttf",
            r"C:\Windows\Fonts\tahoma.ttf",
            r"C:\Windows\Fonts\malgun.ttf",
            r"C:\Windows\Fonts\seguisym.ttf",
        ];
        let loaded: Vec<Vec<u8>> = candidates
            .iter()
            .filter_map(|p| std::fs::read(p).ok())
            .collect();
        if loaded.is_empty() {
            eprintln!("跳过：本机读不到字体文件（非 Windows 环境？）");
            return;
        }

        let mut missing: Vec<char> = Vec::new();
        for &c in &charset {
            if c.is_ascii() || c.is_whitespace() {
                continue;
            }
            if !loaded.iter().any(|bytes| font_has_glyph(bytes, c)) {
                missing.push(c);
            }
        }
        assert!(
            missing.is_empty(),
            "越南语列里有 {} 个字符在字体链里没有字形（会显示成 □）：{:?}",
            missing.len(),
            missing
        );
    }

    /// 在字体文件里查"这个码点有没有字形"——**真正解析 `cmap` 表**。
    ///
    /// ⚠️ **本函数是 2026-09-27 重写的，旧实现是个假阳性工厂。** 旧实现在整个字体
    /// 字节流里**直接搜该码点的 UTF-16BE 两字节**，靠"这两个字节出现过"猜字形存在。
    /// 它有多离谱：对 `msyh.ttc`（中文字体）报"有"格拉哥里 `U+2C00`、吠陀扩展
    /// `U+1CD0`、卢恩 `U+16A0` —— 一个中文字体不可能有这些字形，纯粹是那两个字节
    /// 恰好出现在文件的别处。**结果就是拿它做判据的测试全部空转**（越南语那条"每个
    /// 字符都有字形"能通过，不是因为字形齐，而是因为这个函数几乎对什么都点头）。
    /// 它还被反过来用错过一次：本轮探针据此得出"天城文 128/128 全覆盖"的结论，
    /// 与 `I18N_REMAINING_LANGUAGES.md` §3.1 的真实实测（四个候选字体全缺）矛盾——
    /// 矛盾逼出了这次重写。
    ///
    /// 现在的实现走真正的表结构：`ttcf`（.ttc 集合）→ sfnt offset table → `cmap` →
    /// 选一个 Unicode 子表（优先 `(3,10)` UCS-4，其次 `(3,1)` BMP，再次 platform 0）
    /// → 在 format 4 / format 12 里查，并且**要求 glyphId != 0**（0 是 `.notdef`，
    /// 那等于没有）。
    ///
    /// **解析不动时返回 `false`**（保守方向）：宁可把"其实有"报成没有（测试变红、
    /// 有人去看一眼），也不要报成有（测试静默通过、缺陷留下）。这条与本项目对
    /// "判据宁可假红不可假绿"的一贯要求一致。
    fn font_has_glyph(font_bytes: &[u8], c: char) -> bool {
        fn u16_at(b: &[u8], i: usize) -> Option<u16> {
            b.get(i..i + 2).map(|s| u16::from_be_bytes([s[0], s[1]]))
        }
        fn u32_at(b: &[u8], i: usize) -> Option<u32> {
            b.get(i..i + 4)
                .map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
        }

        fn lookup(font_bytes: &[u8], c: char) -> Option<bool> {
            // ① sfnt 起点：.ttc 是集合文件，真正的 offset table 在 `ttcf` 头之后
            let sfnt = if font_bytes.starts_with(b"ttcf") {
                u32_at(font_bytes, 12)? as usize
            } else {
                0
            };

            // ② 表目录里找 `cmap`
            let num_tables = u16_at(font_bytes, sfnt + 4)? as usize;
            let mut cmap = None;
            for i in 0..num_tables {
                let rec = sfnt + 12 + i * 16;
                if font_bytes.get(rec..rec + 4).is_some_and(|s| s == b"cmap") {
                    cmap = Some(u32_at(font_bytes, rec + 8)? as usize);
                    break;
                }
            }
            let cmap = cmap?;

            // ③ 选一个 Unicode 子表
            let n = u16_at(font_bytes, cmap + 2)? as usize;
            let mut sub = None;
            let mut best = 0u8;
            for i in 0..n {
                let rec = cmap + 4 + i * 8;
                let pid = u16_at(font_bytes, rec)?;
                let eid = u16_at(font_bytes, rec + 2)?;
                let off = cmap + u32_at(font_bytes, rec + 4)? as usize;
                let rank = match (pid, eid) {
                    (3, 10) => 3,
                    (3, 1) => 2,
                    (0, _) => 1,
                    _ => 0,
                };
                if rank > best {
                    best = rank;
                    sub = Some(off);
                }
            }
            let sub = sub?;

            let cp = c as u32;
            match u16_at(font_bytes, sub)? {
                // format 4：BMP，段查找（endCode / startCode / idDelta / idRangeOffset）
                4 => {
                    if cp > 0xFFFF {
                        return Some(false);
                    }
                    let cp = cp as u16;
                    let seg_x2 = u16_at(font_bytes, sub + 6)? as usize;
                    let seg = seg_x2 / 2;
                    let end_base = sub + 14;
                    let start_base = end_base + seg_x2 + 2;
                    let delta_base = start_base + seg_x2;
                    let range_base = delta_base + seg_x2;
                    for i in 0..seg {
                        let end = u16_at(font_bytes, end_base + i * 2)?;
                        if cp > end {
                            continue;
                        }
                        let start = u16_at(font_bytes, start_base + i * 2)?;
                        if cp < start {
                            return Some(false);
                        }
                        let delta = u16_at(font_bytes, delta_base + i * 2)?;
                        let range = u16_at(font_bytes, range_base + i * 2)?;
                        let glyph = if range == 0 {
                            cp.wrapping_add(delta)
                        } else {
                            let at =
                                range_base + i * 2 + range as usize + (cp - start) as usize * 2;
                            match u16_at(font_bytes, at)? {
                                0 => return Some(false),
                                g => g.wrapping_add(delta),
                            }
                        };
                        return Some(glyph != 0);
                    }
                    Some(false)
                }
                // format 12：UCS-4，分段表
                12 => {
                    let groups = u32_at(font_bytes, sub + 12)? as usize;
                    for i in 0..groups {
                        let g = sub + 16 + i * 12;
                        let lo = u32_at(font_bytes, g)?;
                        let hi = u32_at(font_bytes, g + 4)?;
                        let start_glyph = u32_at(font_bytes, g + 8)?;
                        if cp >= lo && cp <= hi {
                            return Some(cp - lo + start_glyph != 0);
                        }
                    }
                    Some(false)
                }
                // 其它 format（0/6/13…）本测试用不到 —— 保守报 false，等有人真需要时再补。
                _ => Some(false),
            }
        }

        lookup(font_bytes, c).unwrap_or(false)
    }

    /// 每列**条目数必须相同**（宏保证了编译期一致，这里再钉一次运行期长度）。
    #[test]
    fn every_language_column_has_the_same_shape() {
        let expected = Language::ALL.len();
        for (name, texts) in ALL {
            assert_eq!(
                texts.len(),
                expected,
                "「{name}」有 {} 列，但 Language::ALL 是 {expected} 种语言",
                texts.len()
            );
        }
    }

    /// `ALL` 的列顺序必须与 [`Language::ALL`] 一致，且 [`LANGS`] 是它的大写镜像。
    ///
    /// 这是三处（`Language::ALL` / `LANGS` / 表里 `langs:`）的**唯一机器校验点**：
    /// 顺序错位会让整个界面显示成另一种语言，而且**编译能过**，所以必须测。
    ///
    /// 名字比较要**先去掉下划线**：表里写的 `ZH_HANT` 是给人看的（可读），而
    /// `format!("{:?}")` 对 `ZhHant` 给出 `"ZhHant"`（Rust 的派生 Debug 按大小写
    /// 边界分词，但**不插下划线**），大写后是 `"ZHHANT"`。两者语义相同、写法不同。
    #[test]
    fn langs_table_matches_language_order() {
        assert_eq!(
            LANGS.len(),
            Language::ALL.len(),
            "LANGS 的项数与 Language::ALL 不一致"
        );
        let normalize = |s: &str| s.replace('_', "").to_uppercase();
        for (symbol, language) in LANGS.iter().zip(Language::ALL) {
            let expected = format!("{language:?}");
            assert_eq!(
                normalize(symbol),
                normalize(&expected),
                "LANGS 里的 {symbol:?} 与 Language::{language:?} 对不上——列顺序错位了"
            );
        }
    }

    /// 每种语言的 `strings()` 必须指向**不同的**表（否则切了语言界面不变）。
    #[test]
    fn every_language_variant_maps_to_a_distinct_table() {
        let mut seen: Vec<(*const Strings, Language)> = Vec::new();
        for language in Language::ALL {
            let ptr = language.strings() as *const Strings;
            if let Some((_, other)) = seen.iter().find(|(p, _)| *p == ptr) {
                panic!("{language:?} 与 {other:?} 指向同一张表——strings() 里漏改了");
            }
            seen.push((ptr, *language));
        }
        assert_eq!(seen.len(), Language::ALL.len());
    }

    #[test]
    fn language_picks_the_matching_table() {
        let zh = Language::Zh.strings();
        let en = Language::En.strings();

        assert_eq!(zh.menu_file, "文件");
        assert_eq!(en.menu_file, "File");
        assert_eq!(zh as *const Strings, &ZH as *const Strings);
        assert_eq!(en as *const Strings, &EN as *const Strings);
        assert_ne!(
            zh as *const Strings, en as *const Strings,
            "两种语言必须指向不同的表"
        );

        // 抽查每一种都取到非空且与中文不同的表（英文/中文本身排除）。
        for language in Language::ALL {
            let table = language.strings();
            assert!(
                !table.menu_file.is_empty(),
                "{language:?} 的 menu_file 是空串"
            );
        }
        assert_eq!(Language::Ja.strings().menu_file, "ファイル");
        assert_eq!(Language::De.strings().menu_file, "Datei");
    }

    /// 中文列必须与 F3 之前硬编码的原文**逐字一致**（最显眼的那几条）。
    ///
    /// 全量比对做不到（原文散在 8 个文件里、已经删掉了），所以这里只钉住用户
    /// 一眼就会看出来的几条——它们要是变了，"切回中文界面没变化"这个验收标准
    /// 就已经破了。
    #[test]
    fn chinese_column_keeps_the_original_wording() {
        let s = Language::Zh.strings();

        assert_eq!(s.sidebar_toggle, "☰ 侧边栏");
        assert_eq!(s.receive_settings, "接收设置:");
        assert_eq!(s.send_area, "发送区:");
        assert_eq!(s.send_title, "发送");
        assert_eq!(s.open_port_button, "打开串口");
        assert_eq!(s.close_port_button, "关闭串口");
        assert_eq!(s.timestamp_prefix_line, "时间戳前缀: {}");
        assert_eq!(s.waiting_data, "等待数据…");
        assert_eq!(s.none_placeholder, "（无）");
        assert_eq!(s.empty_line, "<空行>");
        assert_eq!(s.ending_none, "无");
        assert_eq!(
            s.prefs_note,
            "以上均为用户偏好，改动即时生效并在退出时自动保存"
        );
        assert_eq!(
            s.encoding_gbk_note,
            "GBK 与 GB18030 的接收结果相同（共用解码器），仅发送时的可表示字符范围不同"
        );
    }

    /// 简体中文列必须仍是**中文为主**（防止整列被替换成别的语言）。
    ///
    /// 不逐条断言"必须含中文"——`hex` / `ending_lf` 这类本来就该是 ASCII。
    #[test]
    fn chinese_column_is_overwhelmingly_chinese() {
        let total = ALL.len();
        let with_cjk = ALL
            .iter()
            .filter(|(_, texts)| texts.first().is_some_and(|zh| has_cjk(zh)))
            .count();
        assert!(
            with_cjk * 10 >= total * 8,
            "中文列只有 {with_cjk}/{total} 条含中文，像是被整列替换过了"
        );
    }

    /// 每个非中文列都必须**看得出与中文不同**——不能整列复制中文。
    ///
    /// 允许个别条目相同（"HEX" / "UDP" / "LF" 这类专有名词本来就一样），
    /// 但整列重合度过高就说明"复制中文列当译文"。
    #[test]
    fn non_chinese_columns_are_actually_translated() {
        for (index, language) in Language::ALL.iter().enumerate() {
            if matches!(language, Language::Zh | Language::ZhHant) {
                // 繁体本就大量沿用简体（术语一致），不按同一条阈值卡。
                continue;
            }
            let total = ALL.len();
            let same_as_zh = ALL
                .iter()
                .filter(|(_, texts)| texts.first() == texts.get(index))
                .count();
            assert!(
                same_as_zh * 2 < total,
                "{language:?} 列有 {same_as_zh}/{total} 条与中文**一字不差**，\
                 像是没翻译直接复制了中文列"
            );
        }
    }

    /// 繁体列必须**是真正的繁体**：与简体列有实质差异，且用到了简体列里**不存在的**
    /// 字形。
    ///
    /// 这条最初写的是"繁体与简体**高度重合**（≥50%）"——**那个前提是错的**，实测
    /// 199 条里只有 22 条一字不差。原因不是翻译出问题，而是繁体本地化**本来就会
    /// 改术语**：`设置`→`設定`、`字节`→`位元組`、`发送区`→`傳送區`。
    /// 199 条里只剩 22 条与简体相同，而这些**全是**纯 ASCII/数字/符号条目
    /// （`UDP` / `LF` / `HEX` / `ms（{}~{}）`）——它们当然一个字符都不用改。
    ///
    /// 所以正确的判据不是"重合率"，而是**差异的规模**。而且**不能**靠一张写死的
    /// "繁体专属字表"：那个列表极易夹进简繁共用的字（`位` `元` `單` `開` 都不是
    /// 繁体专属），阈值也就失去意义（实测写死的表在故意把整列改回简体时**仍能通过**）。
    ///
    /// 这里改成**从数据自证**：
    /// 1. 两列不能完全相同；
    /// 2. 统计"只出现在繁体列、简体列里一个都没有"的字符个数——这是**真正**的
    ///    简→繁转换证据，不受写死字表的影响。故意把整列抄成简体时，这个数会掉到 0。
    #[test]
    fn traditional_chinese_is_a_real_traditional_locale() {
        let zh_index = 0;
        let hant_index = Language::ALL
            .iter()
            .position(|l| *l == Language::ZhHant)
            .expect("ZhHant 必须在 Language::ALL 里");

        let zh_column: String = ALL.iter().map(|(_, t)| t[zh_index]).collect();
        let hant_column: String = ALL.iter().map(|(_, t)| t[hant_index]).collect();

        assert_ne!(
            zh_column, hant_column,
            "繁体列与简体**完全**相同，说明没有真正做繁体（至少「設定」「傳送」这类要改）"
        );

        // 只出现在繁体列的字 —— 简→繁转换的直接证据。
        let zh_set: std::collections::BTreeSet<char> = zh_column.chars().collect();
        let hant_only: String = hant_column
            .chars()
            .filter(|c| !zh_set.contains(c) && has_cjk(&c.to_string()))
            .collect();
        let distinct: std::collections::BTreeSet<char> = hant_only.chars().collect();

        // 实测基线：繁体列里有 273 处、31 个不同字形是简体列没有的（`傳`/`設`/`檢`/
        // `視`/`檔`/`連`/`錄`/`顯` …）。阈值取 15 个不同字形——远低于真实值，
        // 但足以在"整列被换回简体"（会掉到接近 0）时判红。
        assert!(
            distinct.len() >= 15,
            "繁体列里只有 {} 个简体列没有的汉字（{:?}）——\
             像是把简体列抄了一份，没有真正转换字形",
            distinct.len(),
            distinct
        );
    }

    /// 俄语列必须**真的用西里尔字母**，波兰语列必须**真的用波兰语变音字母**。
    ///
    /// 与 [`script_allowed`] 的西里尔条目是一对，缺一不可：
    /// - `script_allowed` 保证"**别的**语言列里没混进西里尔"；
    /// - 但它管不住"**俄语列里一个西里尔都没有**"——整列抄成英文/波兰语照样能过。
    ///   本轮实测证实了这个空缺：把 `ru` 列写成波兰语，旧检测器集合全部不命中。
    /// - 这里从**数据自证**：俄语列必须有足够多的西里尔字符、波兰语列必须有
    ///   `ą ć ę ł ń ó ś ź ż` 这类拉丁扩展字母。
    ///
    /// 与 [`vietnamese_column_uses_latin_extended`] 同一思路：判据来自被检查的
    /// 数据本身，不依赖写死的字符表（写死的表极易夹进共用字符而失去意义）。
    #[test]
    fn third_batch_columns_use_their_own_alphabets() {
        /// 波兰语**独有**的变音字母——即不出现在其它任何一种语言列里的字母。
        ///
        /// 实测（解析全部 15 列后取差集）：`ą ć ę ł ń ś ż` 这 7 个在别的列里一个都没有，
        /// 出现在 **101** 条波兰语译文里。
        ///
        /// **为什么不用完整的波兰字母表**（`ą ć ę ł ń ó ś ź ż`）：`ó` 与 `ź` 会出现在
        /// 别的罗曼语系列里（葡萄牙语 `não`、西班牙语 `Versión`），拿它们当判据，
        /// 整列被换成西语也照样命中。判据必须是**差异集**，不是"看起来像波兰语"。
        fn has_polish_only_letter(text: &str) -> bool {
            text.chars().any(|c| "ąćęłńśżĄĆĘŁŃŚŻ".contains(c))
        }

        let index_of = |want: Language| {
            Language::ALL
                .iter()
                .position(|l| *l == want)
                .unwrap_or_else(|| panic!("Language::ALL 里必须有 {want:?}"))
        };
        let count_in = |index: usize, pred: fn(&str) -> bool| {
            ALL.iter()
                .filter(|(_, texts)| texts.get(index).is_some_and(|t| pred(t)))
                .count()
        };

        // 俄语：西里尔条数必须够多（实测 195/199 条含西里尔）。
        // 阈值取 100——远低于真实值，但整列被换成拉丁语种时会掉到 0。
        let ru_cyrillic = count_in(index_of(Language::Ru), has_cyrillic);
        assert!(
            ru_cyrillic >= 100,
            "俄语列只有 {ru_cyrillic}/{} 条含西里尔字母——像是抄了拉丁语种的列",
            ALL.len()
        );

        // 波兰语：独有变音字母条数（实测 101 条）。
        // 阈值取 40：低于真实值但仍高于"整列被换成其它拉丁语种"时的 0。
        // 这条同时保证 `pl` 不是俄语列——西里尔列不会命中它。
        let pl_only = count_in(index_of(Language::Pl), has_polish_only_letter);
        assert!(
            pl_only >= 40,
            "波兰语列只有 {pl_only}/{} 条含 `ą ć ę ł ń ś ż` 这类波兰独有字母——\
             像是丢了变音，或整列被换成了别的拉丁语种",
            ALL.len()
        );

        // 土耳其语是纯拉丁（Ç ğ ı İ ö ş ü），不做字形归属断言；
        // 但必须与中文列有实质差异，防"整列复制中文"。
        let tr_index = index_of(Language::Tr);
        let same_as_zh = ALL
            .iter()
            .filter(|(_, texts)| texts.first() == texts.get(tr_index))
            .count();
        assert!(
            same_as_zh * 4 < ALL.len(),
            "土耳其语列有 {same_as_zh}/{} 条与中文一字不差",
            ALL.len()
        );
    }

    /// `fill` 的基本行为，以及最常见的坑：实参不足 / 实参过多。
    #[test]
    fn fill_substitutes_placeholders_in_order() {
        assert_eq!(fill("a{}b{}c", &[&1, &2]), "a1b2c");
        assert_eq!(fill("无占位符", &[]), "无占位符");
        assert_eq!(fill("{}", &[&"甲"]), "甲");

        assert_eq!(
            fill("{} 与 {}", &[&1]),
            "1 与 {}",
            "实参不足时留下未填充的 {{}}，而不是 panic"
        );
        assert_eq!(fill("只有 {}", &[&1, &2]), "只有 1", "多余的实参被忽略");
    }

    /// 每条模板的 `{}` 个数必须与**实际调用处**传的实参数一致——**全部 12 种语言**。
    ///
    /// 这件事在单语时期只要比"中文和英文一样"就够了；现在语种一多，最容易出的错
    /// 是**某一种语言的译文把 `{}` 漏掉或复制成两个**（翻译时手抖，或对着别的
    /// 语种复制粘贴）。`fill()` 是纯位置替换，占位符个数错了不会 panic，只会在
    /// 界面上显示一个孤零零的 `{}` 或吞掉一个参数——所以必须逐语种钉死。
    ///
    /// 表按**字段名**索引（不是按值），这样每个语种都从自己的表里取值，
    /// 而不是只检查被硬写进来的两种。
    fn pinned_placeholder_counts() -> &'static [(&'static str, usize)] {
        &[
            ("will_send_bytes", 1),
            ("unmappable_in_encoding", 1),
            ("hex_parse_failed", 1),
            ("version_label", 1),
            ("settings_dir", 1),
            ("port_label", 1),
            ("target_label", 1),
            ("local_port_line", 1),
            ("baud_label", 1),
            ("baud_custom_hint", 2),
            ("rx_bytes_label", 1),
            ("tx_bytes_label", 1),
            ("pending_bytes_label", 1),
            ("force_wrap_hover", 1),
            ("send_file_progress", 2),
            ("preset_full", 1),
            ("preset_count_text", 2),
            ("display_mode_line", 1),
            ("timestamp_prefix_line", 1),
            ("logged_bytes", 1),
            ("output_dir_line", 1),
            ("chart_summary", 3),
            ("ignored_fields", 1),
            ("discarded_segments", 1),
            ("chart_format_hint", 1),
            ("send_history_count", 2),
            ("send_history_count_text", 2),
            ("repeat_interval_range", 2),
            ("samples_range", 2),
            ("chart_count_text", 2),
            ("next_file_name", 1),
            ("logging_written", 1),
            ("font_scale_preview", 1),
            ("filter_count", 2),
        ]
    }

    #[test]
    fn template_placeholder_counts_are_pinned() {
        let placeholders = |template: &str| template.matches("{}").count();

        // 表里的字段名必须真实存在，否则下面的查找会静默跳过（＝测试空转）。
        for &(field, _) in pinned_placeholder_counts() {
            assert!(
                ALL.iter().any(|(name, _)| *name == field),
                "钉住的字段名 {field:?} 在文案表里不存在——测试会空转"
            );
        }

        for &(field, expected) in pinned_placeholder_counts() {
            let (_, columns) = ALL
                .iter()
                .find(|(name, _)| *name == field)
                .expect("上面刚断言过存在");
            assert_eq!(
                columns.len(),
                Language::ALL.len(),
                "{field:?} 的列数与语种数不符"
            );
            for (lang, text) in Language::ALL.iter().zip(columns.iter()) {
                assert_eq!(
                    placeholders(text),
                    expected,
                    "{:?} 的 {field:?} 译文 {{}} 个数不对（应为 {expected}）：{text:?}",
                    lang.endonym()
                );
            }
        }
    }

    /// 反向：不带占位符的文案里**任何语种**都不该冒出 `{}`。
    #[test]
    fn templates_without_placeholders_have_none_in_any_language() {
        let placeholders = |template: &str| template.matches("{}").count();
        let plain = [
            "menu_file",
            "settings_title",
            "sec_receive",
            "send_title",
            "waiting_data",
            "none_placeholder",
            "copy_all_log",
            "filter_toggle",
            "filter_hint",
            "filter_clear",
            "filter_note",
            "pending_bytes_hover",
            "log_config_locked_hint",
            "send_file",
            "send_file_cancel",
            "force_wrap",
        ];
        for field in plain {
            let (_, columns) = ALL
                .iter()
                .find(|(name, _)| *name == field)
                .unwrap_or_else(|| panic!("字段 {field:?} 不存在"));
            for (lang, text) in Language::ALL.iter().zip(columns.iter()) {
                assert_eq!(
                    placeholders(text),
                    0,
                    "{:?} 的 {field:?} 不该有占位符：{text:?}",
                    lang.endonym()
                );
            }
        }
    }

    /// 把一行代码里的字符串字面量内容都抠出来（先去掉行注释，再逐字符扫，
    /// 反斜杠转义跳两个字符）。不需要正经词法分析：这几个文件里既没有块注释，
    /// 也没有 `r"..."` 原始串——`tests::scanned_ui_files_have_no_block_comments` 挡住后者。
    fn string_literals(source: &str) -> Vec<String> {
        let mut found = Vec::new();

        for line in source.lines() {
            let code = match line.find("//") {
                Some(pos) => &line[..pos],
                None => line,
            };

            let chars: Vec<char> = code.chars().collect();
            let mut i = 0;
            while i < chars.len() {
                if chars[i] != '"' {
                    i += 1;
                    continue;
                }
                i += 1;
                let mut literal = String::new();
                while i < chars.len() {
                    match chars[i] {
                        // 跳过转义序列（\" / \n / \u{4e2d} 都按两个字符处理）
                        '\\' => i += 2,
                        '"' => {
                            i += 1;
                            break;
                        }
                        c => {
                            literal.push(c);
                            i += 1;
                        }
                    }
                }
                found.push(literal);
            }
        }

        found
    }

    /// 纯 UI 源码里**不许**再出现硬编码中文。
    ///
    /// 这是 F3 之后唯一能长期兜住"新加的控件又写死中文"的机制：文案表本身保证不了
    /// 这一点，只有"绕开文案表就失败"能保证。文件清单**只收纯 UI 文件**——
    /// `app.rs` / `receive.rs` / `log_file.rs` 里按设计保留中文提示行（它们会写进
    /// 实时日志与导出文件，见模块文档），所以不能按同一条规则卡。
    ///
    /// 多语种（2026-09-24）之后，语言档位的**自名**（"中文（简体）" / "日本語" /
    /// "한국어" …）统一放在 `app.rs` 的 `Language::endonym()` 里——`app.rs` 不在
    /// `PURE_UI_FILES` 内，所以这里已经**不需要任何白名单**；真有中文漏进纯 UI 文件，
    /// 那一定是绕开了文案表与 `endonym()`，应当直接判红。
    #[test]
    fn ui_sources_do_not_hardcode_chinese() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));

        // 前提：自名确实不在被扫描的文件里（否则下面"零白名单"的说法是假的）。
        for relative in PURE_UI_FILES {
            let source = std::fs::read_to_string(root.join(relative)).unwrap_or_else(|e| {
                panic!(
                    "读不到 {relative}（{}）: {e}",
                    root.join(relative).display()
                )
            });
            let body = match source.find("#[cfg(test)]") {
                Some(pos) => &source[..pos],
                None => source.as_str(),
            };
            for language in Language::ALL {
                let endonym = language.endonym();
                assert!(
                    !has_cjk(endonym) || !body.contains(endonym),
                    "{relative} 里直接写了语言自名 {endonym:?}；\
                     应改调 `Language::endonym()`（在 app.rs 里），否则每加一种语言都要改这个地方"
                );
            }
        }

        for relative in PURE_UI_FILES {
            let path = root.join(relative);
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("读不到 {relative}（{}）: {e}", path.display()));

            // 只看 `#[cfg(test)]` 之前的正文：测试里的 `assert!` 消息是给开发者看的，
            // 不是界面文案，不必（也不该）跟着界面语言变。
            let body = match source.find("#[cfg(test)]") {
                Some(pos) => &source[..pos],
                None => source.as_str(),
            };

            for literal in string_literals(body) {
                if !has_cjk(&literal) {
                    continue;
                }
                panic!(
                    "{relative} 里有硬编码中文的字符串字面量：{literal:?}\n\
                     界面文案必须放进 `src/strings.rs` 的文案表（每种语言一列），\
                     语言自名必须走 `Language::endonym()`，\
                     否则切到别的语言时这一处会露出中文。"
                );
            }
        }
    }

    /// 纯 UI 文件清单。`ui_sources_do_not_hardcode_chinese` 只扫这几个。
    const PURE_UI_FILES: &[&str] = &[
        "src/sidebar.rs",
        "src/panels/menu_bar.rs",
        "src/panels/settings.rs",
        "src/panels/info.rs",
        "src/panels/chart.rs",
    ];

    /// `text` 里是否出现了 `needle`，且 `needle` 前面不是标识符字符。
    ///
    /// 用来把**原始字符串**（`r"…"` / `r#"…"#`）与"标识符末尾的 r 后面跟着一个普通
    /// 字符串"区分开——`egui::Panel::right("right_sidebar")` 里就有个 `r"`，但它不是
    /// 原始字符串。没有这层判断，下面的前提检查会天天误报。
    fn contains_token(text: &str, needle: &str) -> bool {
        text.match_indices(needle).any(|(index, _)| {
            !matches!(
                text[..index].chars().next_back(),
                Some(c) if c.is_alphanumeric() || c == '_'
            )
        })
    }

    /// 上面那个扫描器的前提条件：这些文件里不能有块注释与原始字符串，
    /// 否则 `string_literals` 会误判（把块注释里的引号当成字符串、或漏掉原始串
    /// 里的中文）。
    ///
    /// 这条测试把"扫描器可以偷懒"这个前提写成了可执行的断言——前提一旦破了，
    /// 这里会先失败，而不是让 `ui_sources_do_not_hardcode_chinese` 变成一条骗人的绿灯。
    #[test]
    fn scanned_ui_files_have_no_block_comments() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));

        for relative in PURE_UI_FILES {
            let source = std::fs::read_to_string(root.join(relative))
                .unwrap_or_else(|e| panic!("读不到 {relative}: {e}"));

            assert!(
                !source.contains("/*"),
                "{relative} 里出现了块注释，扫描器需要相应升级"
            );
            assert!(
                !contains_token(&source, "r\"") && !contains_token(&source, "r#\""),
                "{relative} 里出现了原始字符串，扫描器需要相应升级"
            );
        }
    }
    /// ⭐ **字形检测器自己必须先证明不会假阳性**（2026-09-27）。
    ///
    /// 这条是拿血换来的。上一版 `font_has_glyph` 在字体字节流里直接搜码点的 UTF-16BE
    /// 两字节，对中文字体 `msyh` 报"有"格拉哥里 / 吠陀扩展 / 卢恩字形——一个中文字体
    /// 不可能有这些。**所有**拿它做判据的测试因此空转（"没有任何字符缺字形"永远成立），
    /// 而且它还被拿去得出了一个**相反的结论**（"天城文 128/128 全覆盖"），与
    /// `I18N_REMAINING_LANGUAGES.md` §3.1 的真实实测（四个候选字体全缺）冲突。矛盾
    /// 逼出了检测器的重写。
    ///
    /// 判据两侧都断：既验"绝不该有的不许报有"，也验"确实有的必须报有"——只断前者会
    /// 放过"谎报无"（那会把真缺陷误报成字体问题，白白折腾人）。最后两个断言用**两个
    /// 字体恰好相反**的样本（汉字 / 泰文在 msyh 与 tahoma 里正好一有一无），那是"它
    /// 真的在读 cmap 表"的直接证据——一个瞎点头或瞎摇头的实现过不了这一关。
    #[test]
    fn the_glyph_probe_is_not_a_yes_machine() {
        let fonts = [
            (r"C:\Windows\Fonts\msyh.ttc", "msyh"),
            (r"C:\Windows\Fonts\tahoma.ttf", "tahoma"),
        ];
        let loaded: Vec<(&str, Vec<u8>)> = fonts
            .iter()
            .filter_map(|(path, name)| std::fs::read(path).ok().map(|b| (*name, b)))
            .collect();
        if loaded.is_empty() {
            eprintln!("跳过：本机读不到字体文件（非 Windows 环境？）");
            return;
        }

        for (c, label) in [
            ('\u{2C00}', "格拉哥里字母"),
            ('\u{1CD0}', "吠陀扩展"),
            ('\u{16A0}', "卢恩字母"),
            ('\u{10330}', "哥特字母"),
            ('\u{12000}', "楔形文字"),
        ] {
            for (name, bytes) in &loaded {
                assert!(
                    !font_has_glyph(bytes, c),
                    "{name} 报出它有{label}的字形——绝无可能，说明字形检测器在假阳性"
                );
            }
        }

        let pick = |want: &str| -> Vec<u8> {
            loaded
                .iter()
                .find(|(n, _)| *n == want)
                .unwrap_or_else(|| panic!("本机读不到 {want}"))
                .1
                .clone()
        };
        let msyh = pick("msyh");
        let tahoma = pick("tahoma");

        assert!(font_has_glyph(&msyh, '中'), "msyh 必须有汉字字形");
        assert!(font_has_glyph(&tahoma, 'A'), "tahoma 必须有拉丁字形");
        assert!(
            !font_has_glyph(&tahoma, '中'),
            "tahoma 里没有汉字——报有就是瞎点头"
        );
        assert!(
            !font_has_glyph(&msyh, '\u{0E01}'),
            "msyh 里没有泰文字形（泰文靠 tahoma）——报有就是瞎点头"
        );
    }

    /// **字体链对候选语言的覆盖**——加 `th` / `ar` / `he` / `hi` 之前必须先看这一条。
    ///
    /// 实测（2026-09-27，用重写后的 [`font_has_glyph`]；字体链 =
    /// msyh / msyhbd / simsun / simhei / tahoma / malgun / seguisym）：
    ///
    /// | 文字 | 字母区覆盖 | 结论 |
    /// |---|---|---|
    /// | 泰文 U+0E01–0E3A | 全 | ✅ 字体**已就绪**（`tahoma.ttf` 提供） |
    /// | 阿拉伯文 U+0621–064A | 全 | ✅ 字体已就绪；但 **bidi 重排**这一层 `epaint` 没有 ⇒ 排出来是**反的**，所以不做 |
    /// | 希伯来文 U+05D0–05EA | 全 | 同上 |
    /// | **天城文 U+0900–097F** | **0** | ❌ **本字体链完全没有**——加 `hi` 必须先挂 `Nirmala UI` / `Mangal` |
    ///
    /// ⚠️ **口径是"字母区"，不是整个 Unicode 区块**——这一点踩过：拿整块
    /// `U+0590–05FF` 算，希伯来文只到 **87/112（77%）**，看着像个缺陷，其实那 25 个
    /// 缺口全是**本来就未分配**的码点：任何字体都不会有它们的字形，拿它们算覆盖率
    /// 只会得到一条永远红的测试。判据要看的是"用户真会用到的那批字符"。
    ///
    /// 这条测试**只断言字体链已经提供的三种**，天城文不进断言：它现在的确是 0，但那
    /// 是"尚未做的先决条件"，不是缺陷——把它断言成"必须缺"等于把缺陷固化成预期，
    /// 将来补上字体反而要改测试。
    ///
    /// 为什么值得留成正式测试：它把"字体够不够"从一次性实测变成持续保证。将来谁动
    /// `main.rs::setup_fonts`（换字体、少挂一个回退档），这里会立刻红，而不是等真加了
    /// 那种语言、用户看到满屏方框才发现。
    #[test]
    fn font_chain_covers_the_scripts_it_already_provides() {
        let candidates = [
            r"C:\Windows\Fonts\msyh.ttc",
            r"C:\Windows\Fonts\msyhbd.ttc",
            r"C:\Windows\Fonts\simsun.ttc",
            r"C:\Windows\Fonts\simhei.ttf",
            r"C:\Windows\Fonts\tahoma.ttf",
            r"C:\Windows\Fonts\malgun.ttf",
            r"C:\Windows\Fonts\seguisym.ttf",
        ];
        let loaded: Vec<Vec<u8>> = candidates
            .iter()
            .filter_map(|path| std::fs::read(path).ok())
            .collect();
        if loaded.is_empty() {
            eprintln!("跳过：本机读不到字体文件（非 Windows 环境？）");
            return;
        }

        // 字母区（不是整块）：辅音 / 字母 / 元音符号这些用户真会用到的字符。
        for (label, lo, hi) in [
            ("泰文", 0x0E01u32, 0x0E3Au32),
            ("阿拉伯文", 0x0621, 0x064A),
            ("希伯来文", 0x05D0, 0x05EA),
        ] {
            let all: Vec<char> = (lo..=hi).filter_map(char::from_u32).collect();
            let covered = all
                .iter()
                .filter(|c| loaded.iter().any(|b| font_has_glyph(b, **c)))
                .count();
            let pct = covered * 100 / all.len();
            assert!(
                pct >= 90,
                "{label}（U+{lo:04X}–{hi:04X}）的字母区在字体链里只覆盖 {covered}/{}（{pct}%）——换过字体链？掉下去之后加该语言会满屏方框 □",
                all.len()
            );
        }
    }
}
