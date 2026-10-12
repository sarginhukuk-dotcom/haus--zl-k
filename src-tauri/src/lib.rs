//! Haus Culture Özlük Otomasyonu — Tauri kabuğu.
//! Sayfa (dist/index.html) tek dosyalık HTML programdır; bu kabuk yalnızca
//! kayıtları Belgeler\<Firma>\veri.json içine güvenle yazar, belgeleri
//! Belgeler klasörüne kaydeder, menü ve tek-pencere davranışını sağlar.

mod disk;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::json;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

use disk::Disk;

#[derive(Deserialize, Clone)]
struct Firma {
    urun_adi: String,
    klasor_adi: String,
    surum: String,
}

struct Durum {
    _kilit: Option<std::fs::File>,
    disk: Arc<Disk>,
    firma: Firma,
    #[allow(dead_code)]
    son_belge: Mutex<Option<Instant>>,
}

const BOOT_JS: &str = r#"
(function(){
  var boot = __BOOT__;
  var hata = null;
  function rep(e){ if(hata){ try{ hata(String((e && e.message) || e)); }catch(_){} } }
  function inv(c, a){
    try { return window.__TAURI_INTERNALS__.invoke(c, a); } catch(e){ return Promise.reject(e); }
  }
  // Sayfa yeniden yüklenirse (location.reload) gömülü ilk veri eskimiştir; son durum
  // oturum belleğinden alınır ve diske yeniden gönderilir.
  var veri = boot.veri, cur = {}, fark = false;
  try {
    var snap = sessionStorage.getItem('__haus_snap');
    if(snap){
      sessionStorage.removeItem('__haus_snap');
      var o = JSON.parse(snap);
      if(o && typeof o === 'object'){ veri = o; fark = true; }
    }
  } catch(_){}
  Object.keys(veri).forEach(function(k){ cur[k] = String(veri[k]); });
  function snapAl(){ try{ sessionStorage.setItem('__haus_snap', JSON.stringify(cur)); }catch(_){} }
  window.addEventListener('pagehide', snapAl);
  window.addEventListener('beforeunload', snapAl);
  window.hausDisk = {
    load: function(){ return veri; },
    set: function(k, v){ cur[String(k)] = String(v); inv('disk_set', { key: String(k), value: String(v) }).catch(rep); },
    remove: function(k){ delete cur[String(k)]; inv('disk_remove', { key: String(k) }).catch(rep); },
    onError: function(cb){ hata = cb; },
    info: function(){ return boot.info; }
  };
  window.__hausHata = rep;
  if(fark){
    var gonder = function(){
      var eski = boot.veri || {};
      Object.keys(cur).forEach(function(k){ if(String(eski[k]) !== cur[k]) inv('disk_set', { key: k, value: cur[k] }).catch(rep); });
      Object.keys(eski).forEach(function(k){ if(!(k in cur)) inv('disk_remove', { key: k }).catch(rep); });
    };
    window.addEventListener('DOMContentLoaded', function(){ setTimeout(gonder, 50); });
  }

  // Belge indirme: FileSaver'ın saveAs çağrısı Belgeler klasörüne kaydedilir.
  function b64(blob){
    return new Promise(function(res, rej){
      var r = new FileReader();
      r.onload = function(){ var s = String(r.result); res(s.slice(s.indexOf(',') + 1)); };
      r.onerror = function(){ rej(r.error || new Error('okunamadı')); };
      r.readAsDataURL(blob);
    });
  }
  function kaydet(blob, ad){
    if(!(blob instanceof Blob)) blob = new Blob([blob]);
    return b64(blob)
      .then(function(d){ return inv('belge_kaydet', { name: String(ad || 'belge'), dataB64: d }); })
      .then(function(msg){ try{ if(typeof toast === 'function') toast(msg); }catch(_){} })
      .catch(function(e){ try{ alert('Belge kaydedilemedi: ' + String((e && e.message) || e)); }catch(_){} });
  }
  var hook = function(blob, ad){ kaydet(blob, ad); };
  try {
    Object.defineProperty(window, 'saveAs', { configurable: true, get: function(){ return hook; }, set: function(){} });
  } catch(_){ window.saveAs = hook; }

  // Üst çubukta (tarayıcıdaki gibi) İndirilenler klasörüne götüren simge.
  function indSimge(){
    var h = document.querySelector('header.app');
    if(!h || document.getElementById('hcIndBtn')) return;
    var b = document.createElement('button');
    b.id = 'hcIndBtn'; b.type = 'button'; b.title = 'İndirilen belgeleri göster';
    b.setAttribute('aria-label', 'İndirilenler klasörünü aç');
    b.innerHTML = '<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3v12"/><path d="m7 11 5 5 5-5"/><path d="M5 20h14"/></svg>';
    b.style.cssText = 'margin-left:auto;background:rgba(255,255,255,.1);border:1px solid rgba(255,255,255,.22);color:#fff;border-radius:8px;width:38px;height:38px;display:flex;align-items:center;justify-content:center;cursor:pointer;flex:none';
    b.onmouseenter = function(){ b.style.background = 'rgba(255,255,255,.2)'; };
    b.onmouseleave = function(){ b.style.background = 'rgba(255,255,255,.1)'; };
    b.onclick = function(){ inv('indirilenler_ac', {}).catch(rep); };
    h.appendChild(b);
  }
  if(document.readyState === 'loading') window.addEventListener('DOMContentLoaded', indSimge); else indSimge();

  // Dış bağlantılar (WhatsApp vb.) varsayılan tarayıcıda açılır.
  window.open = function(u){
    if(u && /^https?:/i.test(String(u))) inv('dis_ac', { url: String(u) }).catch(rep);
    return null;
  };
})();
"#;

fn boot_betigi(disk: &Disk, firma: &Firma) -> String {
    let boot = json!({
        "veri": disk.veri_json(),
        "info": {
            "klasor": disk.kok.to_string_lossy(),
            "not": disk.acilis_notu,
            "surum": firma.surum,
        }
    });
    BOOT_JS.replace("__BOOT__", &serde_json::to_string(&boot).unwrap_or_else(|_| "{\"veri\":{},\"info\":{}}".into()))
}

// ---------- Sistem işlemleri ----------

fn indirilenler_klasoru(app: &AppHandle) -> Option<PathBuf> {
    if let Ok(d) = app.path().download_dir() {
        return Some(d);
    }
    std::env::var("USERPROFILE").ok().map(|u| PathBuf::from(u).join("Downloads"))
}

#[tauri::command]
fn indirilenler_ac(app: AppHandle, st: tauri::State<'_, Durum>) {
    match indirilenler_klasoru(&app) {
        Some(k) if k.exists() => klasor_ac(&k),
        _ => klasor_ac(&st.disk.belge_dir),
    }
}

fn klasor_ac(p: &Path) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer").arg(p).spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("xdg-open").arg(p).spawn();
    }
}

#[allow(dead_code)]
fn dosyada_goster(p: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Boşluklu yollar için argüman Gezgin'e olduğu gibi (tırnaklı) iletilmeli.
        let _ = std::process::Command::new("explorer").raw_arg(format!("/select,\"{}\"", p.display())).spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = p;
    }
}

/// Yalnızca https, http ve mailto; kullanıcı adı/parola içeren adresler reddedilir.
fn url_ac(url: &str) -> Result<(), String> {
    let u = tauri::Url::parse(url).map_err(|e| e.to_string())?;
    match u.scheme() {
        "https" | "http" | "mailto" => {}
        _ => return Err("izin verilmeyen adres".into()),
    }
    if !u.username().is_empty() || u.password().is_some() {
        return Err("izin verilmeyen adres".into());
    }
    #[cfg(windows)]
    {
        std::process::Command::new("rundll32")
            .arg("url.dll,FileProtocolHandler")
            .arg(u.as_str())
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("xdg-open").arg(u.as_str()).spawn().map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn gezinme_izni(url: &tauri::Url) -> bool {
    match url.scheme() {
        "tauri" | "asset" | "ipc" | "blob" | "data" | "about" => true,
        "http" | "https" => {
            let h = url.host_str().unwrap_or("");
            if matches!(h, "tauri.localhost" | "ipc.localhost" | "asset.localhost" | "localhost" | "127.0.0.1") {
                true
            } else {
                let _ = url_ac(url.as_str());
                false
            }
        }
        "mailto" => {
            let _ = url_ac(url.as_str());
            false
        }
        _ => false,
    }
}

// ---------- Komutlar ----------

#[tauri::command]
fn disk_set(key: String, value: String, st: tauri::State<'_, Durum>) {
    st.disk.set(key, value);
}

#[tauri::command]
fn disk_remove(key: String, st: tauri::State<'_, Durum>) {
    st.disk.remove(&key);
}

#[tauri::command]
fn belge_kaydet(name: String, data_b64: String, app: AppHandle, st: tauri::State<'_, Durum>) -> Result<String, String> {
    use base64::Engine;
    let bayt = base64::engine::general_purpose::STANDARD
        .decode(data_b64.as_bytes())
        .map_err(|e| e.to_string())?;
    let yol = match st.disk.belge_kaydet(&name, &bayt, indirilenler_klasoru(&app).as_deref()) {
        Ok(y) => y,
        Err(e) => {
            st.disk.gunluge_yaz(&format!("belge_kaydet HATA {}: {}", name, e));
            return Err(e);
        }
    };
    st.disk.gunluge_yaz(&format!("belge_kaydet tamam {} ({} bayt)", yol.display(), bayt.len()));
    let yedek = yol.to_string_lossy().contains("Elle alinan");
    if yedek {
        Ok(format!("Yedek kaydedildi: {}", yol.display()))
    } else {
        Ok(format!("İndirilenler klasörüne kaydedildi: {}", yol.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()))
    }
}

#[tauri::command]
fn dis_ac(url: String) -> Result<(), String> {
    url_ac(&url)
}

// ---------- Menü ----------

fn mesaj(app: &AppHandle, baslik: &str, metin: &str, tur: MessageDialogKind) {
    app.dialog().message(metin).title(baslik).kind(tur).blocking_show();
}

fn menu_kur(app: &AppHandle) -> tauri::Result<()> {
    let dosya = Submenu::with_items(
        app,
        "Dosya",
        true,
        &[
            &MenuItem::with_id(app, "veri_ac", "Veri klasörünü aç", true, None::<&str>)?,
            &MenuItem::with_id(app, "belge_ac", "İndirilenler klasörünü aç", true, None::<&str>)?,
            &MenuItem::with_id(app, "yedek_ac", "Yedekler klasörünü aç", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "geri_yukle", "Yedekten geri yükle…", true, None::<&str>)?,
            &MenuItem::with_id(app, "yedek_simdi", "Şimdi yedek al", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "cikis", "Çıkış", true, None::<&str>)?,
        ],
    )?;
    let yardim = Submenu::with_items(
        app,
        "Yardım",
        true,
        &[&MenuItem::with_id(app, "hakkinda", "Hakkında", true, None::<&str>)?],
    )?;
    app.set_menu(Menu::with_items(app, &[&dosya, &yardim])?)?;
    app.on_menu_event(|app, ev| {
        let id = ev.id().as_ref().to_string();
        let app = app.clone();
        let st = app.state::<Durum>();
        match id.as_str() {
            "veri_ac" => klasor_ac(&st.disk.kok),
            "belge_ac" => indirilenler_ac(app.clone(), st.clone()),
            "yedek_ac" => klasor_ac(&st.disk.yedek_dir),
            "cikis" => app.exit(0),
            "yedek_simdi" => {
                std::thread::spawn(move || {
                    let st = app.state::<Durum>();
                    let _ = st.disk.hemen_yaz();
                    st.disk.yedek_gerekirse(true);
                    mesaj(&app, "Yedek", &format!("Yedek alındı.\n{}", st.disk.yedek_dir.display()), MessageDialogKind::Info);
                });
            }
            "hakkinda" => {
                std::thread::spawn(move || {
                    let st = app.state::<Durum>();
                    mesaj(
                        &app,
                        &st.firma.urun_adi,
                        &format!(
                            "{}\nSürüm {}\nVeri klasörü: {}\n\nVeriler bu bilgisayarda saklanır; internete gönderilmez.",
                            st.firma.urun_adi,
                            st.firma.surum,
                            st.disk.kok.display()
                        ),
                        MessageDialogKind::Info,
                    );
                });
            }
            "geri_yukle" => {
                std::thread::spawn(move || geri_yukle_akisi(&app));
            }
            _ => {}
        }
    });
    Ok(())
}

fn geri_yukle_akisi(app: &AppHandle) {
    let st = app.state::<Durum>();
    let _ = st.disk.hemen_yaz();
    let secim: Option<PathBuf> = app
        .dialog()
        .file()
        .set_title("Geri yüklenecek yedeği seçin")
        .set_directory(&st.disk.yedek_dir)
        .add_filter("Yedek", &["json"])
        .blocking_pick_file()
        .and_then(|p| p.into_path().ok());
    let Some(yol) = secim else { return };
    let secilen = match disk::dosyadan_oku(&yol) {
        Some(v) if v.keys().any(|k| k.ends_with("hc_personel")) => v,
        _ => {
            mesaj(app, "Geçersiz yedek", "Seçilen dosya bu programın bir yedeği değil.", MessageDialogKind::Error);
            return;
        }
    };
    let onay = app
        .dialog()
        .message("Mevcut veriler seçilen yedekle DEĞİŞTİRİLECEK.\n\nGeri yüklemeden önce mevcut veri ayrıca yedeklenir. Devam edilsin mi?")
        .title("Geri yükleme")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom("Geri yükle".into(), "Vazgeç".into()))
        .blocking_show();
    if !onay {
        return;
    }
    match st.disk.geri_yukle(secilen) {
        Ok(()) => app.restart(),
        Err(e) => mesaj(app, "Geri yükleme başarısız", &e, MessageDialogKind::Error),
    }
}

// ---------- Başlatma ----------

fn hata_bildir(app: &AppHandle, m: &str) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.eval(&format!(
            "window.__hausHata&&window.__hausHata({})",
            serde_json::to_string(m).unwrap_or_else(|_| "\"hata\"".into())
        ));
    }
}

pub fn run() {
    let firma: Firma = serde_json::from_str(include_str!("../firma.json")).expect("firma.json okunamadı");

    let mut b = tauri::Builder::default();
    b = b.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.unminimize();
            let _ = w.set_focus();
        }
    }));
    b = b.plugin(tauri_plugin_dialog::init());

    let firma_k = firma.clone();
    b.invoke_handler(tauri::generate_handler![disk_set, disk_remove, belge_kaydet, dis_ac, indirilenler_ac])
        .setup(move |app| {
            let mut adaylar: Vec<PathBuf> = Vec::new();
            match std::env::var("HAUS_VERI_KLASORU") {
                Ok(v) if !v.is_empty() => adaylar.push(PathBuf::from(v)),
                _ => {
                    // OneDrive'a yönlendirilmiş Belgeler yerine sistem sürücüsünde sabit klasör (ilk tercih).
                    #[cfg(windows)]
                    {
                        let surucu = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
                        adaylar.push(PathBuf::from(format!("{}\\", surucu)).join(&firma_k.klasor_adi));
                    }
                    if let Ok(d) = app.path().document_dir() {
                        adaylar.push(d.join(&firma_k.klasor_adi));
                    }
                    if let Ok(u) = std::env::var("USERPROFILE") {
                        adaylar.push(PathBuf::from(u).join("Documents").join(&firma_k.klasor_adi));
                    }
                    if let Ok(u) = std::env::var("LOCALAPPDATA") {
                        adaylar.push(PathBuf::from(u).join(&firma_k.klasor_adi));
                    }
                    if let Ok(d) = app.path().app_local_data_dir() {
                        adaylar.push(d.join(&firma_k.klasor_adi));
                    }
                }
            }
            // Verisi daha önce oluşmuş klasör önce denenir; uyarı yalnızca ilk geçişte gösterilir.
            let ilk_aday = adaylar.first().cloned();
            adaylar.sort_by_key(|k| !k.join("veri.json").exists());
            let mut hatalar: Vec<String> = Vec::new();
            let mut yedek_not = String::new();
            let mut secilen: Option<PathBuf> = None;
            for (i, k) in adaylar.iter().enumerate() {
                // Önceki sürümlerden kalmış veri varsa o klasör tercih edilir.
                match Disk::hazirla(k) {
                    Ok(()) => {
                        secilen = Some(k.clone());
                        let _ = i;
                        if Some(k) != ilk_aday.as_ref() && !k.join("veri.json").exists() {
                            yedek_not = format!("Belgeler klasörüne yazılamadığı için veriler şu klasörde tutuluyor:\n{}\n\n(Ayrıntı: {})", k.display(), hatalar.join("; "));
                        }
                        break;
                    }
                    Err(e) => hatalar.push(format!("{} → {}", k.display(), e)),
                }
            }
            let kok = match secilen {
                Some(k) => k,
                None => {
                    let metin = format!("Veri klasörü hazırlanamadı:\n\n{}", hatalar.join("\n"));
                    let h = app.handle().clone();
                    std::thread::spawn(move || {
                        mesaj(&h, "Program açılamadı", &metin, MessageDialogKind::Error);
                        h.exit(1);
                    });
                    return Ok(());
                }
            };
            let kilit = match Disk::kilitle(&kok) {
                Ok(f) => Some(f),
                Err(e) if e == "kilitli" => {
                    let metin = format!(
                        "Bu veri klasörü başka bir Haus Culture penceresi tarafından kullanılıyor.\n\nAçık duran başka bir pencere ya da eski sürüm olabilir; onu kapatıp programı yeniden açın.\n\nKlasör: {}",
                        kok.display()
                    );
                    let h = app.handle().clone();
                    std::thread::spawn(move || {
                        mesaj(&h, "Program açılamadı", &metin, MessageDialogKind::Error);
                        h.exit(1);
                    });
                    return Ok(());
                }
                Err(_) => None, // kilit alınamıyorsa (örn. bulut klasörü) çalışmaya devam edilir
            };
            let disk = Arc::new(Disk::ac(kok, &firma_k.surum));
            let script = boot_betigi(&disk, &firma_k);
            let acilis_notu = disk.acilis_notu.clone();
            let kok_goster = disk.kok.display().to_string();
            app.manage(Durum {
                _kilit: kilit,
                disk: disk.clone(),
                firma: firma_k.clone(),
                son_belge: Mutex::new(None),
            });

            let mut wb = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title(firma_k.urun_adi.clone())
                .inner_size(1440.0, 900.0)
                .min_inner_size(1050.0, 680.0)
                .center()
                .initialization_script(&script)
                .on_navigation(|u| gezinme_izni(u));

            #[cfg(debug_assertions)]
            if let Ok(p) = std::env::var("HAUS_TEST_SCRIPT") {
                if let Ok(js) = std::fs::read_to_string(p) {
                    let calisti = std::sync::atomic::AtomicBool::new(false);
                    wb = wb.on_page_load(move |wv, payload| {
                        if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished)
                            && !calisti.swap(true, std::sync::atomic::Ordering::SeqCst)
                        {
                            let _ = wv.eval(&js);
                        }
                    });
                }
            }

            wb.build()?;

            // Arka plan yazıcısı: bekleyen değişiklikler 300 ms sonra atomik olarak yazılır.
            let h = app.handle().clone();
            let d = disk.clone();
            std::thread::spawn(move || {
                let mut son_bildirim: Option<Instant> = None;
                loop {
                    std::thread::sleep(Duration::from_millis(100));
                    if let Err(m) = d.tick() {
                        if son_bildirim.map_or(true, |t| t.elapsed() > Duration::from_secs(30)) {
                            son_bildirim = Some(Instant::now());
                            hata_bildir(&h, &m);
                        }
                    }
                }
            });

            menu_kur(app.handle())?;

            if !yedek_not.is_empty() {
                let h = app.handle().clone();
                let n = yedek_not.clone();
                std::thread::spawn(move || mesaj(&h, "Veri klasörü", &n, MessageDialogKind::Warning));
            }
            if !acilis_notu.is_empty() {
                let h = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(1200));
                    mesaj(&h, "Veri kurtarma", &format!("{}\n\nKlasör: {}", acilis_notu, kok_goster), MessageDialogKind::Warning);
                });
            }

            #[cfg(debug_assertions)]
            if let Ok(ms) = std::env::var("HAUS_TEST_EXIT_MS") {
                if let Ok(ms) = ms.parse::<u64>() {
                    let h = app.handle().clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_millis(ms));
                        h.exit(0);
                    });
                }
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("uygulama başlatılamadı")
        .run(|app, ev| {
            if let RunEvent::Exit = ev {
                if let Some(st) = app.try_state::<Durum>() {
                    let _ = st.disk.hemen_yaz();
                    st.disk.yedek_gerekirse(true);
                }
            }
        });
}
