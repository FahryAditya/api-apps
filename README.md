# Backend API Kas OSIS (Rust Axum + MongoDB) 🦀

Migrasi penuh dari backend lama (Node.js/Express) ke **Rust** menggunakan framework modern **Axum 0.8**, runtime asinkron **Tokio 1.x**, dan driver **MongoDB** resmi.

Seluruh logika bisnis, routing, query filter, agregasi laporan arus kas, serta alur pembayaran QRIS Gateway telah direplikasi 1-to-1 dengan performa dan keamanan memori khas Rust.

---

## 🌟 Fitur & Logika Bisnis yang Dipertahankan

### 1. Anggota (`/api/members`)
- **Sanitasi Title Case Otomatis**: Nama anggota otomatis diformat Title Case saat dibuat atau diperbarui.
- **Proteksi Duplikasi Nama**: Mencegah pendaftaran nama siswa yang sama untuk anggota aktif.
- **Perhitungan Status Lunas Dinamis**: Endpoint `GET /api/members?period=YYYY-MM&search=...` otomatis menghitung status pelunasan kas (`isPaid`, `paidAmount`, `paidTransactionId`, `paidDate`), total lunas (`paidCount`), dan belum lunas (`unpaidCount`).
- **Detail Riwayat Anggota**: `GET /api/members/:id` mengembalikan data profil lengkap beserta riwayat transaksi kas masuk (`kasHistory`).
- **Bulk Import**: `POST /api/members/bulk` untuk import massal dari spreadsheet/Excel.
- **Soft Delete**: `DELETE /api/members/:id` menonaktifkan siswa (`isActive: false`) tanpa menghapus riwayat audit pembayarannya.

### 2. Transaksi Kas (`/api/transactions`)
- **Proteksi Anti-Duplikasi Bertingkat**: Mencegah pencatatan dobel kas masuk untuk anggota yang sama pada minggu/periode yang telah dibayar (`allocatedWeeks` atau `period`).
- **Pencatatan Single & Bulk**:
  - `POST /api/transactions/kas-payment` untuk pembayaran kas perorangan.
  - `POST /api/transactions/kas-payment/bulk` untuk melunasi seluruh anggota aktif yang belum bayar dalam 1 klik massal.
- **Filter Fleksibel & Pagination**: Filter berdasarkan `type` (in/out), `category`, `month` (YYYY-MM), dan `search` teks (keterangan, pencatat, catatan).
- **Auto-Populate Member**: Hasil transaksi otomatis memuat informasi nama dan jabatan siswa (`memberId: { name, role }`).

### 3. Laporan Rekapitulasi Arus Kas (`/api/reports/summary`)
- Agregasi pipeline MongoDB untuk:
  - Saldo Awal (`initialBalance`), Total Pemasukan (`totalIncome`), Total Pengeluaran (`totalExpense`), dan Saldo Akhir (`finalBalance`).
  - Arus kas bulan aktif (`monthIncome`, `monthExpense`).
  - Breakdown kategori untuk Donut Chart (`categoryBreakdown`).
  - Rasio persentase pemasukan vs pengeluaran & surplus kas.

### 4. Konfigurasi Sistem (`/api/settings`)
- Konfigurasi parameter kas sekolah (`orgName`, `treasurerName`, `initialBalance`, `kasFee`, `categoriesIn`, `categoriesOut`).
- Manajemen kategori pemasukan dan pengeluaran (`POST /api/settings/categories`, `DELETE /api/settings/categories/:name`).

### 5. Pembayaran QRIS / Midtrans Gateway (`/api/payment`)
- `POST /api/payment/create-order`: Pembuatan order QRIS dengan proteksi jadwal bayar ganda, order ID unik (`KAS-{timestamp}-{hex}`), dan kalkulasi biaya admin.
- `GET /api/payment/status/:orderId`: Cek status pembayaran real-time (otomatis menandai expired setelah 15 menit).
- `POST /api/payment/simulate/:orderId`: Simulasi pembayaran instan (sandbox demo) yang otomatis menerbitkan transaksi kas resmi ke database.
- `POST /api/payment/webhook`: Webhook listener untuk notifikasi settlement Midtrans.

---

## ⚙️ Konfigurasi Environment (`.env`)

File [`.env`](file:///c:/Users/ACER/Desktop/rust/.env) otomatis dimuat saat startup:
```env
PORT=5000
MONGODB_URI=mongodb+srv://<username>:<password>@cluster.mongodb.net/kas_osis?retryWrites=true&w=majority
DB_NAME=kas_osis
CORS_ORIGIN=*

# Midtrans Payment Gateway Configuration
MIDTRANS_MERCHANT_ID=your_merchant_id
MIDTRANS_CLIENT_KEY=your_client_key
MIDTRANS_SERVER_KEY=your_server_key
MIDTRANS_IS_PRODUCTION=true
```

---

## 🚀 Menjalankan Server

```powershell
# Jalankan development server
cargo run

# Kompilasi rilis optimal (production binary)
cargo build --release
```

Server berjalan di: `http://localhost:5000`

---

## 📖 Ringkasan Endpoint

| Endpoint | Method | Keterangan |
| :--- | :--- | :--- |
| `/` | `GET` | Status Root API |
| `/api/health` | `GET` | Health check koneksi backend |
| `/api/settings` | `GET`, `PUT` | Ambil & perbarui pengaturan kas |
| `/api/settings/categories` | `POST` | Tambah kategori baru (`in` / `out`) |
| `/api/settings/categories/:name` | `DELETE` | Hapus kategori (`?type=in` atau `out`) |
| `/api/members` | `GET`, `POST` | Daftar anggota (+ status kas) & tambah anggota |
| `/api/members/bulk` | `POST` | Import anggota massal |
| `/api/members/:id` | `GET`, `PUT`, `DELETE` | Detail (+ riwayat kas), update, soft-delete anggota |
| `/api/transactions` | `GET`, `POST`, `DELETE` | Filter transaksi, buat transaksi umum, reset transaksi |
| `/api/transactions/kas-payment` | `POST` | Bayar kas perorangan (single member) |
| `/api/transactions/kas-payment/bulk` | `POST` | Bayar kas massal semua siswa yang belum lunas |
| `/api/transactions/:id` | `PUT`, `DELETE` | Perbarui / hapus transaksi |
| `/api/reports/summary` | `GET` | Ringkasan saldo, surplus, & agregasi Donut Chart |
| `/api/payment/create-order` | `POST` | Buat order pembayaran QRIS baru |
| `/api/payment/status/:orderId` | `GET` | Cek status order pembayaran QRIS |
| `/api/payment/simulate/:orderId` | `POST` | Simulasi bayar order (auto-insert transaksi) |
| `/api/payment/webhook` | `POST`, `GET` | Webhook notifikasi Payment Gateway Midtrans |
