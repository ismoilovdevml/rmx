# Tezkor O'chirish
Rewritten alternative rm in rust

[Manuchehr Usmonov](https://github.com/yetimdasturchi)ning C dasturlash tilida yozilgan [rm alternativenidan](https://t.me/yetimdasturchi/1191) ilhomlangan holda Rust dasturlash tilida qayta yozilgan rm alternative dasturi


Ushbu dastur katta fayllarni tezkor o'chirish uchun mo'ljallangan


## Dasturni test qilib ko'rish

Keling dasturni test qilib ko'ramiz

Buning uchun bizga katta hajmdagi juda ko'p fayllar kerak xo'sh buni qayerdan olamiz. Biz fake fayl genereatsiya qiladigan kod yozamiz shell scriptda

Fake fayl generatsiya qiluvchi kod quyidagicha

```bash
#!/bin/bash
for (( i=0; i <= 1000; ++i ))
do
 tmpfile=$(mktemp /tmp/fake/abc-script.XXXXXXXXXXXXXXXXXXXXXXXXXX)
 dd if=/dev/urandom of=$tmpfile bs=1M count=$(expr 1 + $RANDOM % 3)
done
```

Agar siz bu fake fayllarni `/tmp/fake/` dan boshqa joyda joylashtirmoqchi bo'lsangiz kodni quyidagiga o'zgartiting

```bash
#!/bin/bash
for (( i=0; i <= 1000; ++i ))
do
 tmpfile=$(mktemp /home/user/Desktop/test/abc-script.XXXXXXXXXXXXXXXXXXXXXXXXXX)
 dd if=/dev/urandom of=$tmpfile bs=1M count=$(expr 1 + $RANDOM % 3)
done
```

Quyidagi kodda men fake fayllarni `user/Desktop/test` jildida saqlashni kiritdim (Linuxda shunaqa path) bu yerda user ga foydalanuvchi nomi yoziladi xolos

Endi fake fayllarni generatsiya qilish uchun dasturchi ishga tushiramiz

```bash
sh ./fake.sh
```

Endi sizda ko'p miqdorda fayllar bor buni endi Rustda yozilgan dastur orqali o'chirib ko'ramiz


Dasturni gitdan klon qilib olamiz

```bash
https://github.com/ismoilovdevml/alternative-rm.git
```

Dasturni ishlatish uchun uni oldin kompilyatsiya qilib olamiz

```bash
cargo build --release
```

Endi es hozir yaratib olgan fake fayllarimizni o'chirib ko'ramiz 

```bash
./target/release/rm /home/user/Desktop/test
```


