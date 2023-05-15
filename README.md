<h2 align="center">RMX</h2>

<h4 align="center">Rewritten alternative rm in Rust</h4>


<h4 align="center">RMX V0.2.0 version</h4>

![alt text](https://github.com/ismoilovdevml/alternative-rm/blob/master/assets/rmx-v0.2.0.png)

![alt text](https://github.com/ismoilovdevml/alternative-rm/blob/master/assets/rmx-v0.2.0-working.png)



[Manuchehr Usmonov](https://github.com/yetimdasturchi)ning C dasturlash tilida yozilgan [rm alternatividan](https://telegra.ph/rm-remove-uchun-qolbola-alternativ-01-07) ilhomlangan holda Rust dasturlash tilida qayta yozilgan rm alternativ dasturi.


`rmx` katta hajmdagi fayllar juda ko'p fayllarni va jildlarni o'chirish uchun Rustda yozilgan `CLI` dasturidir. Ushbu cli dastur, ayniqsa, tizimingizdan katta miqdordagi keraksiz fayllarni o'chirib tashlash orqali xotirani bo'shatish kerak bo'lganda foydalidir. U samarali ishlash uchun parallel ishlov berishdan foydalanadi va o'chirilgan fayllar soni va hajmi haqida malumot taqdim etadi.

<h4 align="center">RMX qanday ishlaydi</h4>

Asosiysi, rmx - bu jildlar bo'ylab harakatlanadigan, fayllar va sub-directorirelarni o'chiradigan rekursiv fayllarni o'chirish dasturi. U faylni o'chirishni parallellashtirish uchun [`rayon`](https://crates.io/crates/rayon) cratesidan foydalanadi va bu uni katta jildlar uchun samarali qiladi. Shuningdek, u o'chirilgan fayllarning umumiy sonini va hajmini hisoblab chiqadi va o'chirish operatsiyasidan so'ng sizga qisqacha ma'lumot beradi.

Dasturning asosiy funktsiyasi cli argumentlarini o'qiydi, ularning haqiqiyligini tekshiradi va `commands` modulidagi `execute_command` funksiyasini chaqiradi. Bu funksiya berilgan buyruqni izohlaydi va tegishli amalni bajaradi.

<h4 align="center">Foydalanish</h4>

Dasturdan foydalnish uchun tizimingizda Rust o'rnatilgan bo'lishi kerak.

Linux MacOs va unix oilasi uchun o'rnatish
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```
Arch linux
```bash
yay -S rust
```

Dasturni ishga tushirish uchun loyihani githubdan tizmingizga ko'chirib oling

```bash
git clone https://github.com/ismoilovdevml/rmx 
cd rmx
cargo build --release
```

RMX bilan fayllarni o'chirish
```bash
cargo run rmx /faylga/havola/
// namuna
cargo run rmx /home/ismoilovdev/Deesktop/test
```

 
<h4 align="center">Kod sturukturasi</h4>

Kod bazasi bir nechta modullarga ajratilgan:

`main.rs` Bu dasturning kirish nuqtasi. U cli argumentlarini tahlil qiladi va kiritilgan ma'lumotlar asosida tegishli buyruq funktsiyasini chaqiradi. 

`lib.rs` Ushbu faylda dasturning asosiy funksiyalari, jumladan, berilgan havola(path) ostidagi barcha fayllar va jildlarni rekursiv ravishda oʻchirib tashlaydigan `remove_dir_contents` funksiyasi mavjud.

`commands.rs` Ushbu modul berilgan buyruqni talqin qilish va bajarish uchun javobgardir.

`args.rs` Bu modulda cli argumentlarini tahlil qilish funksiyasi mavjud.

`util.rs` Ushbu modul baytlarni o'qilishi mumkin bo'lgan formatga aylantirish kabi yordamchi funktsiyalarni o'z ichiga oladi.


## Dasturni test qilib ko'rish

Keling dasturni test qilib ko'ramiz

Buning uchun bizga katta hajmdagi juda ko'p fayllar kerak xo'sh buni qayerdan olamiz. Biz fake fayl genereatsiya qiladigan kod yozamiz shell scriptda

Fake fayl generatsiya qiluvchi kod quyidagicha

```bash
#!/bin/bash

echo "Number of files to generate"
read files

echo "Enter file path:"
read path

if [ ! -d "$path" ]; then
  mkdir -p "$path"
fi

for (( i=0; i <= $files; ++i ))
do
 tmpfile=$(mktemp $path/abc-script.XXXXXXXXXXXXXXXXXXXXXXXXXX)
 dd if=/dev/urandom of=$tmpfile bs=1M count=$(expr 1 + $RANDOM % 3) status=progress
done
```
Ushbu scriptni ishga tushirish uchun quyidagi buyruqlarni ushbu script turgan jildga terminal orqali kirib yozasiz.

```bash
chmod +x file-generator.sh
./file-generator
```
Dastur sizdan qancha fayl yaratishini so'raydi siz kiritasi masalan 40000-ta.Qancha fayl yaratishni kiritganizdan keyin fayllarni qayerda generatsiya qilishn i so'raydi siz havola(path) berasiz. dastur ishini tugatganidan keyin sizda o'zingiz kiritgan miqdorda fayllar bor buni endi Rustda yozilgan dastur orqali o'chirib ko'ramiz

```bash
cargo run rmx /home/ismoilovdev/Desktop/test/
```
Fake fayllarni genaratsiya qiluvchi dasturning muallifi [Manuchehr Usmonov](https://manu.uno/) va dasturga o'zgartirishlar kiritildi.

