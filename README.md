# Haus Culture Özlük Otomasyonu (Tauri)

Tek dosyalık HTML program (`dist/index.html`) + Tauri kabuğu (`src-tauri`).

- Kayıtlar `Belgeler\Haus Culture\veri.json` içine atomik olarak yazılır (geçici dosya → fsync → doğrulama → yeniden adlandırma).
- Her yazımda önceki sağlam kopya `veri.onceki.json` olarak tutulur; son 30 yedek + eskilerden günde 1 (90 gün) `Yedekler` klasöründe saklanır.
- Ana dosya bozulursa sırayla önceki kopya, sonra en yeni sağlam yedek açılır; bozuk dosya klasörde saklanır.
- İndirilen belgeler `Belgeler\Haus Culture\Belgeler\<tarih>` altına kaydedilir; yedek JSON'ları `Yedekler\Elle alinan` içine.
- Program çevrimdışıdır; dış bağlantılar (WhatsApp, e-posta) varsayılan uygulamada açılır.

## Windows kurulum dosyası

GitHub → **Actions** → **Windows kurulum dosyası** → son çalışma → **Artifacts** → `Haus-Culture-Kurulum`.
Derleme her `main` gönderiminde otomatik çalışır; önce kayıt katmanı testlerini Windows üzerinde çalıştırır.

## Firma uyarlaması

`src-tauri/firma.json` (ürün adı, Belgeler altındaki klasör adı, sürüm) ve `src-tauri/tauri.conf.json` (`identifier`, `productName`) değiştirilir; `dist/index.html` firmaya özel sürümle değiştirilir.
