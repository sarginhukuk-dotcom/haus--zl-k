//! Kayıt katmanı: veri.json (atomik yazma), önceki kopya, rotasyonlu yedekler,
//! bozuk dosyadan kurtarma ve indirilen belgelerin kaydı.
//! Electron sürümündeki (main.js) davranışın birebir karşılığıdır.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::{Local, NaiveDate};
use serde_json::{json, Value};

pub const YAZMA_GECIKMESI: Duration = Duration::from_millis(300);
pub const MIN_YEDEK_ARALIK: Duration = Duration::from_secs(15 * 60);

pub type Veri = BTreeMap<String, String>;

struct Ic {
    veri: Veri,
    kirli: bool,
    son_degisim: Instant,
    son_yedek: Option<Instant>,
    yedekten_beri_degisti: bool,
}

pub struct Disk {
    pub kok: PathBuf,
    pub veri_yolu: PathBuf,
    pub onceki_yolu: PathBuf,
    pub tmp_yolu: PathBuf,
    pub yedek_dir: PathBuf,
    pub belge_dir: PathBuf,
    pub acilis_notu: String,
    surum: String,
    ic: Mutex<Ic>,
}

pub fn damga() -> String {
    Local::now().format("%Y-%m-%d_%H-%M-%S").to_string()
}

/// Dosyadaki veriyi okur; biçim bozuksa ya da değerler metin değilse None döner.
pub fn dosyadan_oku(f: &Path) -> Option<Veri> {
    let metin = fs::read_to_string(f).ok()?;
    let j: Value = serde_json::from_str(&metin).ok()?;
    let obj = j.get("veri")?.as_object()?;
    let mut m = Veri::new();
    for (k, v) in obj {
        m.insert(k.clone(), v.as_str()?.to_string());
    }
    Some(m)
}

fn en_yeni_yedek(yedek_dir: &Path) -> Option<(Veri, String)> {
    let mut l: Vec<String> = fs::read_dir(yedek_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|a| a.starts_with("veri-") && a.ends_with(".json"))
        .collect();
    l.sort();
    l.reverse();
    for a in l {
        if let Some(v) = dosyadan_oku(&yedek_dir.join(&a)) {
            return Some((v, a));
        }
    }
    None
}

/// Dosya adını Windows için güvenli hale getirir.
pub fn guvenli_ad(ad: &str) -> String {
    let son = ad.rsplit(|c| c == '/' || c == '\\').next().unwrap_or("");
    let t: String = son
        .chars()
        .map(|c| if "<>:\"/\\|?*".contains(c) || (c as u32) < 32 { '_' } else { c })
        .collect();
    let t = t.trim().trim_end_matches('.').to_string();
    if t.is_empty() {
        "belge".to_string()
    } else {
        t
    }
}

fn yedek_dosyasi_mi(ad: &str) -> bool {
    let k = ad.to_lowercase();
    k.ends_with(".json") && (k.contains("yedek") || k.contains("geri_donus"))
}

impl Disk {
    /// Klasörleri hazırlar, veriyi (gerekirse kurtararak) yükler ve açılışta bir yedek alır.
    pub fn ac(kok: PathBuf, surum: &str) -> Disk {
        let d0 = Disk {
            veri_yolu: kok.join("veri.json"),
            onceki_yolu: kok.join("veri.onceki.json"),
            tmp_yolu: kok.join("veri.yazilan.tmp"),
            yedek_dir: kok.join("Yedekler"),
            belge_dir: kok.join("Belgeler"),
            kok,
            acilis_notu: String::new(),
            surum: surum.to_string(),
            ic: Mutex::new(Ic {
                veri: Veri::new(),
                kirli: false,
                son_degisim: Instant::now(),
                son_yedek: None,
                yedekten_beri_degisti: false,
            }),
        };
        let _ = d0.klasorleri_hazirla();
        let (veri, not) = d0.yukle();
        let degisti = !veri.is_empty();
        let d = Disk {
            acilis_notu: not,
            ic: Mutex::new(Ic {
                veri,
                kirli: false,
                son_degisim: Instant::now(),
                son_yedek: None,
                yedekten_beri_degisti: degisti,
            }),
            ..d0
        };
        d.yedek_gerekirse(true);
        d
    }

    fn klasorleri_hazirla(&self) -> std::io::Result<()> {
        for d in [&self.kok, &self.yedek_dir, &self.belge_dir] {
            fs::create_dir_all(d)?;
        }
        Ok(())
    }

    fn yukle(&self) -> (Veri, String) {
        if let Some(v) = dosyadan_oku(&self.veri_yolu) {
            return (v, String::new());
        }
        if self.veri_yolu.exists() {
            // veri.json var ama okunamıyor: bozuk dosyayı sakla, önceki kopyaya ya da yedeğe dön
            let _ = fs::copy(&self.veri_yolu, self.kok.join(format!("veri.bozuk-{}.json", damga())));
            if let Some(v) = dosyadan_oku(&self.onceki_yolu) {
                return (v, "Ana veri dosyası okunamadı; bir önceki kayıt açıldı.".into());
            }
            if let Some((v, ad)) = en_yeni_yedek(&self.yedek_dir) {
                return (v, format!("Ana veri dosyası okunamadı; {} yedeği açıldı.", ad));
            }
            return (
                Veri::new(),
                "Ana veri dosyası okunamadı ve yedek bulunamadı. Bozuk dosya klasörde saklandı.".into(),
            );
        }
        (Veri::new(), String::new())
    }

    /// Sayfanın açılışta okuduğu veri (JSON nesnesi).
    pub fn veri_json(&self) -> Value {
        let ic = self.ic.lock().unwrap();
        Value::Object(ic.veri.iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))).collect())
    }

    pub fn set(&self, k: String, v: String) {
        let mut ic = self.ic.lock().unwrap();
        ic.veri.insert(k, v);
        ic.kirli = true;
        ic.son_degisim = Instant::now();
    }

    pub fn remove(&self, k: &str) {
        let mut ic = self.ic.lock().unwrap();
        if ic.veri.remove(k).is_some() {
            ic.kirli = true;
            ic.son_degisim = Instant::now();
        }
    }

    /// Arka plan zamanlayıcısı çağırır: bekleyen değişiklik yeterince eskiyse yazar.
    pub fn tick(&self) -> Result<(), String> {
        let hazir = {
            let ic = self.ic.lock().unwrap();
            ic.kirli && ic.son_degisim.elapsed() >= YAZMA_GECIKMESI
        };
        if hazir {
            self.hemen_yaz()
        } else {
            Ok(())
        }
    }

    /// Atomik yazma: geçici dosya → fsync → doğrula → önceki kopya → yeniden adlandır.
    pub fn hemen_yaz(&self) -> Result<(), String> {
        let mut ic = self.ic.lock().unwrap();
        if !ic.kirli {
            return Ok(());
        }
        let r = self.yaz_ic(&ic.veri);
        match r {
            Ok(()) => {
                ic.kirli = false;
                ic.yedekten_beri_degisti = true;
                drop(ic);
                self.yedek_gerekirse(false);
                Ok(())
            }
            Err(e) => Err(format!(
                "KAYIT BAŞARISIZ: veri diske yazılamadı ({}). Belgeler klasörünü ve disk alanını kontrol edin; Ayarlar → Yedek Al ile yedek indirin.",
                e
            )),
        }
    }

    fn yaz_ic(&self, veri: &Veri) -> Result<(), String> {
        self.klasorleri_hazirla().map_err(|e| e.to_string())?;
        let metin = serde_json::to_string(&json!({
            "surum": 1,
            "program": self.surum,
            "yazildi": Local::now().to_rfc3339(),
            "veri": veri,
        }))
        .map_err(|e| e.to_string())?;
        {
            let mut f = fs::File::create(&self.tmp_yolu).map_err(|e| e.to_string())?;
            f.write_all(metin.as_bytes()).map_err(|e| e.to_string())?;
            f.sync_all().map_err(|e| e.to_string())?;
        }
        if dosyadan_oku(&self.tmp_yolu).is_none() {
            return Err("Yazılan dosya doğrulanamadı".into());
        }
        if self.veri_yolu.exists() && dosyadan_oku(&self.veri_yolu).is_some() {
            let _ = fs::copy(&self.veri_yolu, &self.onceki_yolu);
        }
        fs::rename(&self.tmp_yolu, &self.veri_yolu).map_err(|e| e.to_string())
    }

    /// Yedek alır: değişiklik varsa ve son yedekten 15 dk geçtiyse (ya da zorla).
    pub fn yedek_gerekirse(&self, zorla: bool) {
        let mut ic = self.ic.lock().unwrap();
        if !self.veri_yolu.exists() || (!ic.yedekten_beri_degisti && !zorla) {
            return;
        }
        if !zorla {
            if let Some(t) = ic.son_yedek {
                if t.elapsed() < MIN_YEDEK_ARALIK {
                    return;
                }
            }
        }
        let hedef = self.yedek_dir.join(format!("veri-{}.json", damga()));
        if hedef.exists() {
            return;
        }
        if fs::create_dir_all(&self.yedek_dir).is_ok() && fs::copy(&self.veri_yolu, &hedef).is_ok() {
            ic.son_yedek = Some(Instant::now());
            ic.yedekten_beri_degisti = false;
            drop(ic);
            self.yedekleri_buda();
        }
    }

    /// Son 30 yedek + eskilerden günde 1 (90 güne kadar).
    pub fn yedekleri_buda(&self) {
        let Ok(rd) = fs::read_dir(&self.yedek_dir) else { return };
        let mut l: Vec<String> = rd
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|a| a.len() > 15 && a.starts_with("veri-") && a.ends_with(".json") && a.as_bytes()[9] == b'-' && a.as_bytes()[12] == b'-' && a.as_bytes()[15] == b'_')
            .collect();
        l.sort();
        l.reverse();
        let mut tutulan: std::collections::HashSet<String> = l.iter().take(30).cloned().collect();
        let mut gunler = std::collections::HashSet::new();
        let bugun = Local::now().date_naive();
        for a in l.iter().skip(30) {
            let g = &a[5..15];
            if let Ok(d) = NaiveDate::parse_from_str(g, "%Y-%m-%d") {
                let yas = (bugun - d).num_days();
                if yas <= 90 && gunler.insert(g.to_string()) {
                    tutulan.insert(a.clone());
                }
            }
        }
        for a in &l {
            if !tutulan.contains(a) {
                let _ = fs::remove_file(self.yedek_dir.join(a));
            }
        }
    }

    /// Yedekten geri yükleme: mevcut veri önce yedeklenir, sonra seçilen veri yazılır.
    pub fn geri_yukle(&self, secilen: Veri) -> Result<(), String> {
        let _ = self.hemen_yaz();
        {
            let mut ic = self.ic.lock().unwrap();
            ic.yedekten_beri_degisti = true;
        }
        self.yedek_gerekirse(true);
        let _ = fs::copy(
            &self.veri_yolu,
            self.yedek_dir.join(format!("veri-{}-geri-yukleme-oncesi.json", damga())),
        );
        {
            let mut ic = self.ic.lock().unwrap();
            ic.veri = secilen;
            ic.kirli = true;
        }
        self.hemen_yaz()
    }

    /// İndirilen belgeyi Belgeler\<gün> (yedek JSON'ları Yedekler\Elle alinan) altına kaydeder.
    /// Aynı ad varsa " (2)", " (3)" ... eklenir; hiçbir dosyanın üzerine yazılmaz.
    pub fn belge_kaydet(&self, ad: &str, veri: &[u8]) -> Result<PathBuf, String> {
        let ad = guvenli_ad(ad);
        let klasor = if yedek_dosyasi_mi(&ad) {
            self.yedek_dir.join("Elle alinan")
        } else {
            self.belge_dir.join(&damga()[..10])
        };
        fs::create_dir_all(&klasor).map_err(|e| e.to_string())?;
        let yol = Path::new(&ad);
        let uz = yol.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
        let govde = yol.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "belge".into());
        let mut hedef = klasor.join(&ad);
        let mut n = 1;
        loop {
            match fs::OpenOptions::new().write(true).create_new(true).open(&hedef) {
                Ok(mut f) => {
                    f.write_all(veri).map_err(|e| e.to_string())?;
                    f.sync_all().map_err(|e| e.to_string())?;
                    return Ok(hedef);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    n += 1;
                    hedef = klasor.join(format!("{} ({}){}", govde, n, uz));
                }
                Err(e) => return Err(e.to_string()),
            }
        }
    }
}

#[cfg(test)]
mod testler {
    use super::*;

    fn yeni(d: &Path) -> Disk {
        Disk::ac(d.to_path_buf(), "test")
    }
    fn yaz_bekle(d: &Disk) {
        d.hemen_yaz().unwrap();
    }

    #[test]
    fn yaz_ve_yeniden_ac() {
        let t = tempfile::tempdir().unwrap();
        let d = yeni(t.path());
        d.set("hc_personel".into(), "[1,2]".into());
        d.set("hc_x".into(), "ç ğ ş".into());
        yaz_bekle(&d);
        let d2 = yeni(t.path());
        assert_eq!(d2.veri_json()["hc_personel"], "[1,2]");
        assert_eq!(d2.veri_json()["hc_x"], "ç ğ ş");
        assert!(d2.acilis_notu.is_empty());
    }

    #[test]
    fn remove_kalici() {
        let t = tempfile::tempdir().unwrap();
        let d = yeni(t.path());
        d.set("a".into(), "1".into());
        d.set("b".into(), "2".into());
        yaz_bekle(&d);
        d.remove("a");
        yaz_bekle(&d);
        let d2 = yeni(t.path());
        assert!(d2.veri_json().get("a").is_none());
        assert_eq!(d2.veri_json()["b"], "2");
    }

    #[test]
    fn tick_gecikmeli_yazar() {
        let t = tempfile::tempdir().unwrap();
        let d = yeni(t.path());
        d.set("a".into(), "1".into());
        d.tick().unwrap();
        assert!(!d.veri_yolu.exists(), "gecikme dolmadan yazılmamalı");
        std::thread::sleep(YAZMA_GECIKMESI + Duration::from_millis(50));
        d.tick().unwrap();
        assert!(d.veri_yolu.exists());
    }

    #[test]
    fn bozuk_dosyada_onceki_kopyaya_doner() {
        let t = tempfile::tempdir().unwrap();
        let d = yeni(t.path());
        d.set("k".into(), "eski".into());
        yaz_bekle(&d);
        d.set("k".into(), "yeni".into());
        yaz_bekle(&d);
        fs::write(t.path().join("veri.json"), "{bozuk").unwrap();
        let d2 = yeni(t.path());
        assert_eq!(d2.veri_json()["k"], "eski");
        assert!(d2.acilis_notu.contains("önceki"));
        let bozuk = fs::read_dir(t.path()).unwrap().filter_map(|e| e.ok()).any(|e| e.file_name().to_string_lossy().starts_with("veri.bozuk-"));
        assert!(bozuk, "bozuk dosya saklanmalı");
    }

    #[test]
    fn onceki_de_bozuksa_yedege_doner() {
        let t = tempfile::tempdir().unwrap();
        let d = yeni(t.path());
        d.set("k".into(), "v1".into());
        yaz_bekle(&d);
        d.yedek_gerekirse(true);
        fs::write(t.path().join("veri.json"), "x").unwrap();
        fs::write(t.path().join("veri.onceki.json"), "y").unwrap();
        let d2 = yeni(t.path());
        assert_eq!(d2.veri_json()["k"], "v1");
        assert!(d2.acilis_notu.contains("yedeği açıldı"));
    }

    #[test]
    fn hicbiri_yoksa_bos_ve_not() {
        let t = tempfile::tempdir().unwrap();
        fs::write(t.path().join("veri.json"), "bozuk").unwrap();
        let d = yeni(t.path());
        assert!(d.veri_json().as_object().unwrap().is_empty());
        assert!(d.acilis_notu.contains("yedek bulunamadı"));
    }

    #[test]
    fn yanlis_bicim_reddedilir() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("a.json");
        fs::write(&f, r#"{"veri":{"k":5}}"#).unwrap();
        assert!(dosyadan_oku(&f).is_none());
        fs::write(&f, r#"{"veri":["a"]}"#).unwrap();
        assert!(dosyadan_oku(&f).is_none());
        fs::write(&f, r#"{"veri":{"k":"v"}}"#).unwrap();
        assert!(dosyadan_oku(&f).is_some());
    }

    #[test]
    fn belge_ayni_ad_uzerine_yazmaz() {
        let t = tempfile::tempdir().unwrap();
        let d = yeni(t.path());
        let a = d.belge_kaydet("Deneme_Belge.docx", b"1").unwrap();
        let b = d.belge_kaydet("Deneme_Belge.docx", b"2").unwrap();
        assert_ne!(a, b);
        assert_eq!(fs::read(&a).unwrap(), b"1");
        assert_eq!(fs::read(&b).unwrap(), b"2");
        assert!(b.to_string_lossy().contains("Deneme_Belge (2).docx"));
    }

    #[test]
    fn yedek_json_elle_alinana_gider() {
        let t = tempfile::tempdir().unwrap();
        let d = yeni(t.path());
        let y = d.belge_kaydet("Haus_yedek_2026.json", b"{}").unwrap();
        assert!(y.to_string_lossy().contains("Elle alinan"));
        let b = d.belge_kaydet("Dilekce.docx", b"x").unwrap();
        assert!(b.to_string_lossy().contains("Belgeler"));
    }

    #[test]
    fn guvenli_ad_temizler() {
        assert_eq!(guvenli_ad("..\\..\\x:y?.docx"), "x_y_.docx");
        assert_eq!(guvenli_ad("a/b/c.pdf"), "c.pdf");
        assert_eq!(guvenli_ad(""), "belge");
        assert_eq!(guvenli_ad("..."), "belge");
    }

    #[test]
    fn budama_son_30_korur() {
        let t = tempfile::tempdir().unwrap();
        let d = yeni(t.path());
        for i in 0..40 {
            fs::write(d.yedek_dir.join(format!("veri-2099-01-01_00-00-{:02}.json", i)), "{}").unwrap();
        }
        d.yedekleri_buda();
        let n = fs::read_dir(&d.yedek_dir).unwrap().filter_map(|e| e.ok()).filter(|e| e.file_name().to_string_lossy().starts_with("veri-2099")).count();
        // son 30 + eskilerden aynı güne ait en yeni 1 (Electron sürümüyle aynı kural)
        assert_eq!(n, 31);
    }

    #[test]
    fn geri_yukleme_oncesi_yedek_alir() {
        let t = tempfile::tempdir().unwrap();
        let d = yeni(t.path());
        d.set("k".into(), "simdiki".into());
        yaz_bekle(&d);
        let mut s = Veri::new();
        s.insert("k".into(), "eski".into());
        d.geri_yukle(s).unwrap();
        assert_eq!(yeni(t.path()).veri_json()["k"], "eski");
        let var = fs::read_dir(&d.yedek_dir).unwrap().filter_map(|e| e.ok()).any(|e| e.file_name().to_string_lossy().contains("geri-yukleme-oncesi"));
        assert!(var);
    }
}
