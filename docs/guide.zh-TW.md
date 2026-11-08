# stream-mic 使用說明

[English](guide.en.md) · [回到 README](../README.md)

stream-mic 處理你的麥克風聲音（降噪、EQ、齒音、Gate、壓縮、空間感、限幅），再變成一個虛擬麥克風。OBS 和 Discord
選這個虛擬麥克風就好。遊戲、音樂這些電腦聲音也可以一起送進去。

![stream-mic 的「設備」版面](images/a-zh.png)

> 目前還在開發中，尚未正式發布。

## 需要什麼

**Windows**
- Windows 10 2004 以後的版本，或 Windows 11。收電腦聲音要用到這個版本以後才有的功能。
- VB-CABLE（虛擬音效線）。stream-mic 會把處理好的聲音送到它。Windows 版的壓縮檔裡附了 VB-CABLE 的安裝檔，也可以到
  [vb-cable.com](https://vb-audio.com/Cable/) 下載。

**Linux**
- PipeWire。Ubuntu 22.10、Linux Mint 22 以後預設就是。
- WebKitGTK 4.1（視窗）和 AppIndicator（系統匣）：

      sudo apt install libwebkit2gtk-4.1-0 libayatana-appindicator3-1

**兩邊都要**
- 戴耳機。用喇叭的話，麥克風會收到喇叭放出來的聲音，電腦聲音就會被送出去兩次。

## 安裝

**Windows**
1. 把整個資料夾解壓縮到固定的位置，例如「文件\stream-mic」。`stream-mic.exe` 和 `WebView2Loader.dll` 要放在一起。
2. 安裝 VB-CABLE：
   - 解壓縮 `VBCABLE_Driver_Pack` 壓縮檔。
   - 對 `VBCABLE_Setup_x64.exe` 按右鍵，選「以系統管理員身分執行」，再按 Install Driver。
3. 執行 `stream-mic.exe`。
   - 第一次執行時，Windows 可能會跳出「Windows 已保護您的電腦」。
   - 先按「其他資訊」，再按「仍要執行」。會跳出來是因為這個程式沒有付費的程式碼簽章。

**Linux**

    tar xzf stream-mic-*-linux-*.tar.gz
    ./stream-mic-*-linux/stream-mic

## 開始使用

1. 在「裝置」選你的麥克風和耳機。
2. 選一個模式（螢幕左上的方塊，按 ‹ › 切換）：
   - **說話（全指向）**：給筆電內建麥、領夾麥這類會收進整個房間的麥克風。會補一點厚度，高頻不加亮。
   - **說話（指向）**：給對著嘴的 USB、XLR 麥克風。清晰度和空氣感各加一點。
   - **唱歌**：長音不會被當成噪音。Gate 比較慢、比較淺，有伴奏時會加一點 Reverb。
3. 按 **自動設定**（說明在下一節）。
4. 在 OBS 和 Discord 選虛擬麥克風（見「OBS 和 Discord」一節）。

## 自動設定

![自動設定](images/auto-zh.png)

自動設定會先錄 3 秒房間的聲音，再錄你說話 10 秒：
- **1/2 安靜 3 秒**：先不要說話，音樂和影片也先停掉。這一步是在量房間本身有多吵。
- **2/2 說話 10 秒**：用你直播時的距離和音量說話，可以照著畫面上的文字念。

錄完之後，它只會改三個設定：
- **Gate**：設在房間噪音上方約 10 dB（唱歌模式是 8 dB）。
- **Compressor**：設在你說話時會被壓 3–4 dB 的位置。
- **Makeup**：把音量補到 -18 LUFS。

改過之後，模式名稱旁會標「改過」。不滿意可以按「回到之前」。

遇到下面這些情況，它會告訴你原因，而且不會套用結果：
- 沒聽到人聲。
- 房間的噪音跟人聲差不多大。
- 麥克風太小聲或快要爆音：去調麥克風或錄音介面的 gain，再試一次。
- 安靜時麥克風收到了電腦聲音：可能是錄音介面的 LOOPBACK 開著，或你用的是喇叭。

## 五個旋鈕

| 旋鈕 | 作用 |
| --- | --- |
| **Denoise** | 用 AI 降噪（DeepFilterNet3）去掉風扇聲、鍵盤聲這類穩定的噪音。開了會多大約 40 ms 延遲。安靜的房間不用開。唱歌模式最多只能開到「輕」，開更強會把長音當成噪音。 |
| **厚薄** | 往左比較薄：低頻減少、3 kHz 加一點、低切往上移。往右比較厚：低頻增加、3 kHz 少一點。 |
| **空間感** | 加一點殘響。唱歌模式下，有伴奏時殘響比較多，清唱或聊天時只有三分之一。 |
| **De-ess** | 壓 ㄙ、ㄘ、ㄒ 這類刺耳的齒音，數字越大壓得越多。 |
| **Gate** | 低於這條線的聲音（停頓時的房間噪音）會被調小。旁邊會顯示量到的底噪，Gate 要設在底噪上面。轉到最左邊（-80 dB）就是關。 |

**聽原音**：比較處理前後的聲音。按住空白鍵也可以暫時聽原音，放開就恢復。

## 處理鏈

![處理鏈的 EQ 細節](images/a-eq-zh.png)

畫面下方的處理鏈是聲音實際經過的順序：

    Denoise → EQ → De-esser → Gate → Compressor → Reverb → Limiter

- 每一段上的燈：亮著就是開著。
- 橘色條：這一段現在把聲音壓了多少（GR）。
- 點一段可以看它的細節：
  - 「開」：關掉這一段。
  - 「耳機也要」：這一段要不要放進你耳機聽到的聲音。會造成延遲的段落（例如 Denoise）可以只在送出去的聲音裡處理。

**EQ** 有四個點：
- **Low Cut**：左右拖拉改頻率。
- **Low**（200 Hz）、**Presence**（3 kHz）、**Air**（10 kHz）：上下拖拉改增減。

畫面上的曲線就是實際的濾波效果：
- Low 和 Air 是 shelf（Q 0.707）：中心頻率剛好是增減量的一半，所以點落在曲線上。
- Presence 是鐘形（Q 1）。
- Q 值是固定的。

## 兩種版面

右上角可以切換兩種版面，功能一樣，只是排法不同：
- **A 設備**：像一台硬體效果器，有旋鈕和音量表。
- **B 波形**：顯示最近 10 秒的聲音波形。Gate 和 Compressor 的門檻線可以直接在波形上拖。

![「波形」版面](images/b-zh.png)

右上角的 ⚙ 可以切換版面和語言（繁中、English）。

## 電腦聲音

把遊戲、音樂和人聲一起送出去。

**Windows**
- stream-mic 會自己收電腦裡其他程式的聲音，不用改任何程式的輸出。
- 耳機裡的電腦聲音是 Windows 自己播的，不經過 stream-mic。

**Linux**
- 把遊戲、音樂的輸出改成「stream-mic 電腦聲音」，在系統音效設定或 pavucontrol 改。
- 瀏覽器這類不能選輸出的程式，就把系統的預設輸出設成它。耳機是 stream-mic 直接接的，所以你還是聽得到。

**設定**
- **送進虛擬麥克風**：打開才會跟人聲一起送出去。
- **送出的音量**：送出去的電腦聲音大小。
- **耳機裡的音量**：只影響你自己聽到的大小。

音量表上方會比較人聲和電腦聲音的響度（LUFS）。一般建議電腦聲音比人聲小 6–12 dB，太大或太小都會提示。

> 如果 OBS 另外擷取了「桌面音效」，就把「送進虛擬麥克風」關掉，不然電腦聲音會有兩份。

## 監聽（耳機裡你自己的聲音）

| 選項 | 說明 |
| --- | --- |
| **處理過的** | 耳機裡是處理過的聲音（會延遲的段落會跳過）。Linux 的電腦聲音也是從這裡混進耳機。 |
| **Direct** | 用錄音介面自己的 Direct Monitor（直接監聽）：沒有延遲，但聽到的是沒處理過的聲音。stream-mic 就不再送你的聲音到耳機，免得聽到兩次。 |
| **關** | 耳機裡沒有你自己的聲音。用喇叭時選這個。 |

## OBS 和 Discord

| | Windows | Linux |
| --- | --- | --- |
| OBS：音訊輸入擷取 | CABLE Output (VB-Audio Virtual Cable) | stream-mic |
| Discord：語音設定的輸入裝置 | CABLE Output (VB-Audio Virtual Cable) | stream-mic |

不要在 OBS 或 Discord 裡監聽這個輸入，再把它送回電腦聲音：這樣會形成回授。偵測到回授時，stream-mic 會先暫停電腦聲音，畫面上也會說明。

## 系統匣和開機啟動

- 關掉視窗不會停止處理，stream-mic 會縮到系統匣，直播不會斷。
- 要完全結束：在系統匣圖示的選單選「結束 stream-mic」。
- 系統匣選單也可以打開視窗，或切換聽原音。
- 「裝置」裡可以打開「開機時自動啟動」。開機後只會出現在系統匣，不會開視窗。

設定檔存在：
- Windows：`%APPDATA%\stream-mic\settings.json`
- Linux：`~/.config/stream-mic/settings.json`

**移除**
1. 關掉開機啟動。
2. 從系統匣結束程式。
3. 刪掉程式資料夾和上面的設定資料夾。
4. （Windows）VB-CABLE 要另外移除：再執行一次它的安裝檔，按 Remove Driver。

## 有問題的時候

畫面上方的警告會說明原因：

| 警告 | 怎麼辦 |
| --- | --- |
| 回授 | OBS 或 Discord 在監聽 stream-mic，又被送回電腦聲音。關掉那邊的監聽。 |
| 爆音被圓掉 | 人聲加電腦聲音太大，已經自動處理掉，聽不到破音。常常發生的話，調低「送出的音量」。 |
| 找不到 VB-CABLE（Windows） | 安裝 VB-CABLE。裝好後 stream-mic 會自己接上。 |
| 選的裝置不在 | 先用系統預設的裝置，插回來之後會自動換回去。 |

回報問題時，請附上：發生的時間、你做了什麼，以及畫面截圖。

## 授權

stream-mic 以 Apache License 2.0 授權。用到的第三方元件和它們的授權，列在 NOTICE 和 THIRD_PARTY_LICENSES.txt。

VB-CABLE 是 VB-Audio Software 的軟體，不屬於 stream-mic，也不在 stream-mic 的授權範圍內。
- 來源：www.vb-cable.com
- VB-CABLE is a donationware, all participations are welcome.
- 在公司、學校、機關裡使用時，VB-Audio 要求購買授權。
