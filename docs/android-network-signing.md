# 安卓网络安全因子

依据为已检查的 40.8.0 APK。下面是静态调用链，不代表已取得实际出站请求、独立签名或服务端验收。

## Cronet 回调

`CronetUrlRequest.addSecurityFactor` / `CronetUrlRequestContext.addSecurityFactor` 将交替出现的请求头名称和值转为 HashMap，名称按 Locale.US 转小写，再调用 `onCallToAddSecurityFactor`。同名键按 HashMap.put 替换，并非保留多值列表。

后续链路：

`CronetUrlRequestContext.onCallToAddSecurityFactor` → `X.1Z27.LIZIZ` → `X.1Yzy.LIZIZ` → `X.1Yzc.LJ` → `ICronetAppProvider.onCallToAddSecurityFactor`。提供者管理器还有反射分支，未注册提供者时可以返回 null。

`AbsCronetDependAdapter.onCallToAddSecurityFactor` 将每个头值包装成单项列表，交给 `X.1Ywf.LJIILLIIL`。后者通过 metasec 的 `ms.bd.c.g5` / `h5` 上下文，调用原生 `ms.bd.c.k3.a`。

| 阶段 | 已核对行为 |
| --- | --- |
| 请求入口 | 传入 URL 和转小写后的请求头 Map |
| 原生桥准备 | 从列表取第一个值；名称为 null、列表为 null/空或首值为 null 时跳过；形成交替名称/值数组 |
| 原生调用 | 存在命令号 0x3000001、0x6000001 两个分支，使用 SDK 上下文句柄 |
| 返回 | 原生字符串数组按名称/值对转成 Map，再返回 Cronet |

分支条件中的字符串已通过后述公开样本离线探针解码并与 DEX 分支核对：http 前缀走 0x3000001，ws 前缀走 0x6000001；这不代表具体签名算法已经确定。URL 校验使用小写副本，但传给原生桥的仍是原始 URL；不能据此自行重排查询参数、改写编码或大小写。

这条链路说明早期 Retrofit Request.getHeaders 不是完整出站上下文的证明。它没有证明原生回调具体输出哪些签名头，也没有证明搜索失败由其中某一项导致。

## frameSign 的边界

`MSManager.frameSign` 使用原生命令号 0x200000a；`SecApiImpl.LJIIIZ` 调用该接口时传入类型 2。它与上述 URL/请求头安全因子桥的输入及命令号不同，返回 Map 不能直接当作 HTTP 签名头。

当前独立客户端仍需核对回调注册、SDK 上下文初始化、实际输入输出、原生算法以及最终网络处理。ticket guard 原语、路径模式及配置层组件只覆盖其中一部分。没有通过调用上述原生桥对真实账号重放请求来补证。

## 证据

- Cronet 请求/上下文桥：ev_5ad89c38dbd6ca3075a0d34062506600fee5051614490fb7afe817754731a21a、ev_2bfc9a47c7a8cd44905b982be0e0c9c3b17632e2581c9348a03b372e038dfa8a。
- 回调、监听器及提供者分发：ev_5a73e18c3149df2cd8bdea884f9e21aaa8232b970140a4f1db4f818c8e65b835、ev_0f2021820f4d5b69ded8f909e265c9575a60a205a54da8694aaef6b3aac90fda、ev_2869af304392021b49b3c69fc3d7993cb6bcdce5edb09ae9b1a7bd63d9d117f5。
- 适配器及 metasec 桥：ev_9cb3bce6238b10cfb3e1d4a14d94cea04cb69b010690bafb6731e6df287ec220、ev_3a1d6ba471445179b60d48c33ab6dd20965edc25a4c4fc1526797b53a0930602。
- 帧签名入口与原生声明：ev_1ebc25a42400835dfce361cb73bcb17fb6bf7f4e37aa75f6d526c0a82e58605d、ev_479761c771e61fd48e1a251626437c4d86f9a33b0be553c28b0c3e0901892994、ev_3e434f8decaf183510e52fe4106ce4ac43ec6452d590467a568dab7e6cdb4278。

## 回调注册与上下文

`ms.bd.c.h5.LIZ(long)` 将参数保存在实例字段 LIZ，创建引用该实例的 g5，再将其注册到 X.1Ywf.LJIILJJIL。b3 的静态初始化创建 h5；其 Java 分发方法 LIZLLL 的 0x2000001 分支调用 a3.LIZIZ(long)，再通过虚方法进入 h5.LIZ(long)。0x2000002 分支操作另一实例 l2，不能混为同一个提供者。

以上是 Java 分发入口中的命令号，不能与出站 k3.a 原生调用的命令号混为一个表。该静态链路尚未证明上下文句柄可跨进程或重启复用；保存这个 long 不能替代独立客户端中的 SDK 初始化。

APK 中的 lib/arm64-v8a/libmetasec_ml.so 为 4,530,128 字节的 ELF64 AArch64 文件，SHA-256 为 1fe03703b1f3a806b220271ba2ec1dbed049d8b5f0216b70c0c724d5c5bc027c。动态符号表可见 JNI_OnLoad；这不证明所有注册方法或完整算法已经识别。公开 APK 库仅用于本地调查，未加入项目发行包；未在宿主原生执行该库，也未导入真实运行时句柄；后续有界离线仿真见下文。

注册证据：ev_e62e6c2dbea97eb9e3f768b7a5c2d5db2724fc2fe52ccc4c1d4a3b29a92d181c、ev_09f8f7ad8323930f399d8dbf6c88a39ff766ecb82d0143e3f9e62844c84fcb44。实例初始化和分发证据：ev_628d74acf6ec22cab9fc3c4ef73556b72c44f38dca2c777581e31cd849fb1f94、ev_4cbccfe5d0aff4d88f23d8167c14cf459cda37e28a85cfa6ce0972fa76d43e9d、ev_9b9255981a3b7ea3bc7e9067754c99102096b1ecf260731e9bdd3b933de1110c。


## JNI_OnLoad 静态入口

已按符号范围核对 JNI_OnLoad（RVA 0x2a8bb8，长度 820 字节，证据 ev_65c91068bab823f496bf3571eccc41bb423b6e28e3c82b031ebf63df8c06b595）。入口有读取返回地址并计算间接跳转目标的片段，线性反汇编中被跳过的指令不能直接解释成实际执行行为。没有修改二进制或执行其中的陷阱/系统调用。

在当前文件字节对应的静态路径中，0x1811a4 的辅助函数逐字节 XOR 两个只读输入，写入缓冲区，遇到第二个输入的零字节结束。JNI_OnLoad 为该调用传入 0xa66f8、0x4697a4、0xa6754；解出的十进制字符串为 5256。后续 strtol 和地址运算得到 0x2a8eec - (5256 >> 2) + 0x522 = 0x2a8eec，并将该值作为首参数传给 0x29e764。

0x28dfe8 在这条链路中表现为变参打包辅助函数，不能只因它出现在初始化路径就当作签名分发入口。0x29e764 的完整功能、首参数所指内容、JNI 注册表与方法实现仍未确定；不把候选地址命名成已经识别的虚拟机或签名函数。以上不证明运行时内存没有被初始化代码改写，也不等于动态执行验收。


## 有界离线仿真观察

使用隔离安装的 Unicorn 2.1.4 对原库做 AArch64 仿真，校验库哈希后映射 ELF 段、应用重定位，依次运行 17 项 init_array，再进入 JNI_OnLoad。每阶段限制 500,000 条指令及 3 秒；未知导入、未知系统调用、未建模外部对象访问或越界均停止，不把客体系统调用转发给宿主。

该环境只有虚构的时钟、单线程身份、TLS/互斥量及最小 JavaVM/JNIEnv 接口。类名和父类关系取自同一 APK 的 classes21.dex，不是实际运行的 Android Java 类；没有载入账号、手机私钥或真实设备属性。退出回调仅登记，探针不验证卸载和线程退出行为。原始库未打补丁，宿主环境变量不传入虚构进程。

本次 17 项初始化函数在该环境中返回，随后观察到 RegisterNatives 的注册参数：

| 字段 | 观察值 |
| --- | --- |
| 注册类 | ms/bd/c/k3 |
| 方法 | a |
| 描述符 | (IIJLjava/lang/String;Ljava/lang/Object;)Ljava/lang/Object; |
| 相对库基址地址 | 0x29b2b4 |

探针捕获参数后立即停止，尚未让 RegisterNatives 完成返回，也未证明 JNI_OnLoad 全部完成。此结果定位了统一原生桥的候选实现地址，不证明任何命令分支或 HTTP 签名已执行成功。原生入口还有 JNI 隐含参数，不能把 Java 方法参数直接当作完整原生调用参数。

classes21.dex SHA-256：e86cb997fab9401b5a93c28730999800794d0c53f61ed6623b8ea9ffb9f4f06c；已与原 APK 条目逐字节核对。探针 SHA-256：5e6b774eb55d82443756b32bb6265311679ac0ec3fbefd48f30d7f9b34717f0a。探针及仅含公开注册元数据的输出留在本地研究目录，不作为产品运行时或手机回退通道。

重放增加了 GNU RELRO 与堆/栈不可执行权限检查，注册结果不变。结果文件包含输入及探针哈希、指令/时间上限；研究账本的结果文件证据为 ev_ac797d147a2d6b0c70b051f5e14523c6977e7652008fc19969bec7d9091ce8e7。该证据记录本地虚构环境探针的输出，不是手机实机结果。

## 继续初始化的离线探针

独立探针 `emulate-metasec-bootstrap.py` 允许 RegisterNatives 返回成功后继续执行，当前探针 SHA-256 为 1b9a3a53020d05876bbedc05b44654ca5ac14732fd08880664b57d99cda83c61。在固定 DEX 的方法表和父类关系中，初始化请求的 `MS.b(IIJLjava/lang/String;Ljava/lang/Object;)Ljava/lang/Object;` 解析到 `ms/bd/c/k3.b`。

引用模型为每次创建分配独立句柄，区分局部和全局引用，删除引用后拒绝再次使用。自检覆盖删除局部引用后全局引用仍有效、禁止把全局引用作为局部引用删除、空引用类型为 invalid。该模型取代早期 NewGlobalRef 直接复用局部句柄的简化实现，重放通过原先的 GetObjectRefType 停止点。格式化仅实现当前遇到的单个 `%d`，按 AArch64 va_list 取参数；其他格式停止。NewStringUTF 仅接受 ASCII，其他编码停止；未模拟任何 Java 异常生成操作，遇到未实现操作直接停止。

先前在 CallStaticObjectMethodV 调用 Java `k3.b` 时停止：命令号 `0x1000041`，整数及 long 参数为零，String 和 Object 参数非空。该阶段没有完成 JNI_OnLoad。按 DEX，k3.b 转发 d3.LIZ；该命令位于 d3 的 `(0x1000000, 0x100004a)` 分支，继续交给 b3.LIZLLL。后续核对结果见下。

Java 静态证据：ev_8174cfb704e6ec85d8e2bf8a39543e3cb1fe5ba0b2f99f6ae46d8b8c70b46436、ev_cbea318d44879ce88c38622b23113291ac32c24ce0e097df07115f1871f36452。所有输入仍为公开库、DEX 和虚构环境，无手机、账号或宿主系统调用转发。

进一步核对 b3.LIZLLL：`0x1000041` 是日志回调，整数参数为 0/1/2 时先读取 o2.LIZ；接收器为 null 时直接返回 null。o2 位于 classes.dex，无方法、唯一静态字段为 LIZ；离线探针明确设置日志接收器未注册，不推断手机进程状态。证据：ev_31f26f7285edc922e12842ee89b103b6a25a802c700fe90e538c7c2b8ea578df、ev_ddcd88c482ba0aad44ca8f63a471af84cd47901a1df686278a97141ac9669607。

后续命令 `0x1000000e` 按 d3.LIZ 返回 Long.valueOf(X.0mW8.LIZ)。apkanalyzer 确认该类无方法，静态 volatile LIZ 的编码初值为 2，LIZIZ 为默认零。探针明确采用尚未被 Java 应用修改的字段初值，并仅实现 Long.longValue() 的精确解箱；它们不是实机运行时字段快照，也不能解释为时间戳。

当前停止于 pthread_create：工作线程入口 RVA `0x3a4ccc`，调用方 RVA `0x3a4330`。尚未创建工作线程或返回成功；下一步需执行有界客体线程及其依赖，不能直接跳过。JNI_OnLoad 仍未完成，未建立请求上下文或 HTTP 签名。

## 第一条客体线程调度路径

线程探针 `emulate-metasec-thread.py` 的 SHA-256 为 358b20a37b4518a63c6ee2dcd447730f7bfb313ecc4cb00e095bace9f568defd。保留上述 bootstrap 探针，不覆盖其结果。新探针只执行一条明确的调度：pthread_create 入口暂停父线程，在独立客体栈和 TLS 下执行第一个工作线程，使用不同的合成线程 ID 和线程局部键值。互斥锁现在记录所有者，不能因为 recursive 类型而允许另一个线程递归获取；errno 也使用当前客体 TLS。未创建宿主线程或连接手机。

工作线程在 RVA `0x3a7c9c` 调用 pthread_cond_timedwait：条件状态四字节为零，互斥锁由当前线程持有，深度为 1；绝对期限为 1752147483 秒、654000000 纳秒。该数字来自固定虚构时钟，不是设备时间。探针保存工作线程上下文、释放关联互斥锁并登记等待，不把等待当成已返回。随后写入不可直接解引用的 opaque pthread_t，允许创建调用向父线程返回成功，再继续父线程。若遇到唤醒该等待者的 signal/broadcast，则停止等待实现调度，而非忽略。

当前父线程到达第二次 pthread_create，入口 RVA 为 `0x3a4338`，调用方仍为 `0x3a4330`。第一个线程仍在等待；第二个线程尚未执行。还未覆盖超时、唤醒、join/detach、线程退出析构或其他调度顺序，JNI_OnLoad 仍未完成。下一步须核对第二个入口及其同步依赖。

## 多工作线程与首次唤醒

后续探针 `emulate-metasec-workers.py` 的 SHA-256 为 d7017580440dbaaadaeade1cc633af089c2bcd3e0e3bd546b3e048746fe29401，最多展开 8 个客体线程，每段仍受 500000 指令和 3 秒限制。实际只展开 3 个便停在父线程的 pthread_cond_signal。第二、三个线程入口均为 RVA `0x3a4338`，各自在 `0x3a7bf4` 进入无超时条件等待；等待对象与第一个定时等待线程不同。每次均确认当前线程独占关联互斥锁、深度为 1，登记等待后释放锁，再恢复父线程的创建调用。

目前保留三个等待线程的 CPU 上下文、独立 TLS 键值和等待参数。首次 signal 对应的已登记候选合成线程 ID 为 [2002]。探针在唤醒前停止，未忽略 signal，未让等待函数提前返回；尚需实现通知后的互斥锁重取和线程恢复调度，也未覆盖所有调度顺序。该进展仍不证明 JNI_OnLoad 已返回或请求签名可用。

## 通知、重取锁与初始化返回

当前公开样本探针 `emulate-metasec-decoder.py` 的 SHA-256 为 9ba0ddb2cd7bb6c3e4356fdd341ca05f54c265f32aed12535d80413a5f6c9720。signal 登记候选等待者，父线程释放关联锁后才切换到合成线程 2002；恢复前确认锁已空闲，重新赋予该线程深度为 1 的锁，再让条件等待返回零。恢复后的工作线程在有限指令片段内持续运行，没有返回或遇到新的外部调用。单独扩大到 100 个片段的试验仍未完成；不能据此认定是死循环或计算结束。

为避免单个线程独占客体 CPU，探针保留其上下文，在 8 个附加片段后恢复通知方父线程。父线程随后通知线程 2003，并从 JNI_OnLoad 返回 `0x10006`（JNI 1.6）。这只证明所选虚构环境和调度下的库加载入口返回；工作线程任务仍未完成，不证明 SDK 请求上下文初始化成功。尚未覆盖全部调度、超时、线程退出和 Java 应用启动状态。

在保留待处理通知的有界调用方优先调度下，探针随后调用原生 k3.a，命令号 `0x1000001`，输入为公开 APK 中的 String `3a352a` 和单字节数组 `[0x79]`，返回 Java String `;`。对应原始 b3.LIZLLL 代码将该解码结果追加在包名之间，再移除最后一个分隔符。探针补充了已遇到的 String.getBytes(utf-8)、字节数组长度/区域读取、局部引用容量以及对象类查询；没有实现完整 Java 或 JNI。此结果是字符串解码，不是 HTTP 签名或服务端验收。

当前探针输出的研究账本证据为 ev_cccb8a2c6cc86cc9d3ee805262cd2150923e48e71b69e72f25f2788dac384d44；它记录本地虚构环境的输出，不是手机运行结果。下一步可用同一分支解码 g5 的 URL 条件常量，并将结果与实际 DEX 分支核对。

## 网络桥字符串与分支已核对

网络字符串探针 `emulate-metasec-network-strings.py` SHA-256 为 2988af36761c0a63800517877d5672333df728a9661ca67f2bbdb4e754c4987d，输入清单 SHA-256 为 529e858d9fa97b581a4015659e978ac0ed51815c2cc89cfb17ea3cfd9e3364ef。输入由 classes48.dex 的 X.1Ywf.LJIILLIIL 方法代码偏移 0x6d52cc、相应 array-data payload 提取，并先与原始 APK 中 classes48.dex 逐字节一致性核对。原生库和 classes21.dex 的固定哈希检查继续保留。

| 代码单元偏移 | 用途 | 解码明文 |
| --- | --- | --- |
| 0x164 | contains 检查一 | http |
| 0x16a | contains 检查二 | https |
| 0x172 | contains 检查三 | ws |
| 0x178 | 全部不满足时的异常 | it must be http/https/wss |
| 0x18a | startsWith 检查一 | http |
| 0x190 | startsWith 检查二 | ws |
| 0x196 | WebSocket 分支追加的名称 | URL |

SDK 对 URL 的小写副本依次执行上述 contains 检查，全部不满足才抛异常；它并非严格 URL 解析器。随后 startsWith(http) 使用命令 0x3000001，原始 URL 作为 String 参数，交替头名称/值数组作为 Object 参数。否则 startsWith(ws) 在头数组后追加字面名称 URL 和原始 URL，使用命令 0x6000001，String 参数为 null。两者都传入 h5 的运行时上下文 long。只通过 contains、但两种 startsWith 都不满足时返回空 Map；不自行纠正 URL。这里仍没有实际网络请求或签名验收。

本地探针输出的账本证据为 ev_31a03be46362ba56d7dbc9587014f9f1d0fe708dfde62ae65a75de492df25121。后续 SDK 上下文来源调查确认 i3.LIZ(String) 在已初始化门控后调用 0x4000002 取得 Long，再与 Java 配置表中的 u2 及 Context 构造 f3；不能用任意 long 代替。对应静态证据 ev_7cb91a3502a69748081219071a8bf17904eb02bfc447473670ca3eb49f740f70。下一步核对 i3 的初始化入口及 0x4000002 的前置条件。

## Java 配置初始化前置条件

i3.LIZJ(Context,u2) 校验非空 Context、配置以及 appID（优先 LIZ，空时取 LJI），先调用 i3.LIZIZ 加载阶段；已注册的 appID 返回 false。非空 LJIIIZ 会先送入命令 0x2000011，然后 u2.LIZ() 的 JSON 字符串送入命令 0x4000001；只有 native Boolean 为 true 才将 u2 写入 Java ConcurrentHashMap，并继续日志接收器、广播及异步任务注册。不能把 JNI_OnLoad 返回当作这一步成功。静态证据 ev_91e0bad7b160f51f7770f8553c21b92807251bb3db59a41884993c3c0d86f62d。

加载阶段把 ApplicationContext 存入 v2，再使用命令 0x1000003、null String 和 Context Object 进行原生调用；另有 SwitchManager(21) 控制的 k3.c 分支，尚未实现，不能自动跳过后宣称等价。该阶段最后才设置 Java 已加载标志。证据 ev_202c4658926043e9c08d4f05d9a89bf42e8f936f62cce6451cb9d19bcf889e23。

u2.LIZ 序列化为 17 项 JSON 数组：LIZ、LJII、LJI、LJIIIIZZ、s4.LIZ、LIZIZ、LIZJ、LIZLLL、LJ、LJFF、LJIIJ、LJIIJJI、LJIIL、LJIJI、LJIJJ，再追加两个交替键值数组 LJIILIIL 与 LJIILJJIL。整数先转十进制 String；字符串和 Map 键值经 LIZIZ 转换：仅 String 保留并执行 Java trim，null/其他类型变空 String。Map 仅在原始键 null/空时跳过，不把 trim 后为空等同于原始键为空；没有排序依据。静态证据 ev_909b4c825af7bda079f7a8c72406e414cc36a179521b61f15b0a34e2c47b8d47、ev_eaf7e9f8a13e0be2b47b75aaf287c03835f8b72e1dc7b74318fcc88b5b86436b。各字段语义及实际配置尚待核对。

探针 emulate-metasec-bind-context.py 的 SHA-256 为 3c06ee3b081b51de3483c92352f42233edddec9c3dd0093f4833646359754d80。仅 JNI 加载后执行 0x4000002 返回 null。随后在明确的虚构 Context 下执行 0x1000003，getPackageName 仅返回 invalid.example.metasec_fixture，getenv 继续使用与 environ 一致的空客体环境、不读取宿主；此调用返回 null。之后再次查询 0x4000002 仍返回 null。没有运行 0x4000001 或声称 Context 已完整初始化。该负例说明这组虚构输入下，加载/Context 调用没有产生可用管理器；不推断真实账号状态。输出账本证据 ev_0ed5affe3c9d09b956595e23d9eb4e9101d91c6807092c95651ce903750d369d。下一步核对 u2 的字段构造、实际配置来源和原生配置命令。

## 恢复后的配置构造核对

暂停恢复后原 /tmp/re 研究文件已不存在。重新从已连接手机只读拉取 base.apk，固定 SHA-256 仍为 a4e0682ff174d73dbc00d77622037090ee88f164fc0f9b62c324a1234e64ba57；新缓存与账本位于忽略目录 target/research/metasec。上述旧仿真结果是历史记录，当前尚未恢复原型脚本，不能声称已重新运行。

新的构造调用引用分析确认 u2 的直接创建点为 s2(String,int) 和 X.0mVm(X.0mVu)。s2 继承 u2：将首参写入 LIZ（appID），将 APK 中固定许可证写入 LJIIIIZZ，将整数参数写入 LJIIL；拒绝 null/空/字符串 0 的 appID 或空许可证。X.0mVu 继承 s2；X.0mVm 创建自己的 u2 基类默认字段并持有输入配置引用，尚不能将其理解为完整字段复制。证据 ev_22b86b554f25d5710fbdfd1762c3eef0829525c01d9d32e359fb72b5872ea98d、ev_bd5ca920eb65c3a86cbc107ddb44bb64c2b192cd77b738ae81d0f2cd31e5dd5e、ev_07f31202e0552f11b05371f2d83a89cab4fd0c50c1de03d771454a8aef102ad1。公开许可证原文不加入文档或源码。下一步核对 builder 的继承方法和构建时如何复制配置字段。

## 配置来源与 Rust 常量解码

SecApiImpl.LIZIZ 创建 X.0mVu(appID.toString(), mode)，写入 LJIIJ=0、LJIILJJIL[tk_key]=douyin；调用方提供的 Map 非空时写入 LJIILIIL。X.0mVm 仅保存这个对象引用，MSManagerUtils 初始化时取出同一引用交给 i3.LIZJ，没有把包装对象的基类默认字段当作最终配置。后续按同一 appID 调用 MSManagerUtils.get。证据 ev_6b04b55665548df9cbf09117fd15d19b86a403db9ea1b4fe05b86693408c8ce6。

运行模式由 X.0mVo.LIZ(ZZ) 决定：第二个布尔为真返回 280，其次第一个为真返回 290；否则初始化类型 1/2 返回 810，类型 3/5 在未登录时返回 810，其余路径返回 99999。不能把某次账号状态下的结果硬编码成所有客户端默认模式；这些数值也不等于 ticket-guard 的 TEE/REE/HMAC 选择。证据 ev_8e44a0cdeb0b047486c90210f07cd43460543a0be4751b0e84b313cae6db8656，array payload 另按当前 DEX 核对为 [1,2]。

新增 src/metasec.rs 的 decode_constant，重建零整数/long 参数下公开常量 payload 的字节变换。十字节公开掩码在原生库文件偏移 453964 找到；每个输入字节按十字节槽位选掩码，seed 索引也先对十取模再对 seed UTF-8 长度取模。7 个先前原生探针网络桥样本全部匹配，25 字节错误文本覆盖多次循环；缺失 seed 或非法 UTF-8 返回错误。它只解码公开常量，不运行 SDK 初始化、JNI 或 HTTP 签名。

s4 的静态初始化原本通过命令 0x1000001、seed 443549 解码 19 字节 payload，不能直接从应用版本号填充。Rust 解码器从当前 DEX 原始 payload 推导得到 v04.10.04.01-bugfix。此 SDK 版本字符串尚未重新调用原生库交叉验证，作为推导结果保留，未作为已验收的在线配置写入登录态。s4 静态证据 ev_78c44cde135a2120c82e9e87a8f2704f75937ae799ae9a301c2a5d8ceacefd84。

补充静态交叉核对：同一固定 SHA-256 的 libmetasec_ml.so 中也存在明文 v04.10.04.01-bugfix，与新解码结果一致。这增强了版本常量的静态依据，但仍不等于重新执行原生解码命令或完成 SDK 初始化。打包清单另确认包含 src/metasec.rs，不包含 target/research 中的 APK、DEX、ELF 或研究账本。

## 异步设备补全与配置编码验收

g3.run 转入 LIZ：当 LJIILL 回调存在且主设备字段 LIZJ 为空时，通过 AppLog.getServerDeviceId 取值，并以 appID String 查询命令 0x4000002。取值和上下文均可用时，在配置锁内写入备用字段 LJIJI、标志 LJIJJ=1，再使用该上下文调用 0x200000e，String 为取到的设备 ID、Object 为 Integer(0)。缺少值或上下文时安排 500ms 延迟重试。主字段与备用字段不能混为一项。证据 ev_28c05709d690a6d6a481aaa9a0213a1670650cf30b58940d1bf7188e95a2ae74、ev_3bbdbd5b07bb7a529c9f28e2ec4a3a5902b0c3d51c577e11cbf1d6a3c9996f51。

另确认 MSManager.setDeviceID 写入 u2.LIZJ，句柄不等于 -1 时发出 0x2000002；setInstallID 写入 u2.LJ 并使用 0x2000003。两者的 String 参数为输入标识，Object 参数为 Context。这两个出站原生命令不能与 Java 回调表中相同数字混淆。证据 ev_1ea6880d9a4c69789918f031cf75b91bedbc378c2d0e90ab50862003c0d47ffc、ev_4c893ed083c98ad5309cfca3ad2937cffe1672e0096ab5e1fc4cc7da02c96111。未在真实应用上调用这些 setter。

新增 metasec::Configuration，编码 17 项数组和两个有序键值数组。未映射字符串槽位保留为 opaque_text；整型控制位保留原字段位置，不按猜测命名。默认值对应 u2 构造状态，不代表通过应用初始化门控。Java trim 仅移除两端 U+0000..U+0020，保留 NBSP；先判断原始键是否 null/空，再 trim，其他类型 Map 值变空 String；整型转十进制 String。JSON 输出保留输入 Map 迭代顺序并按 Android JSONStringer 转义斜杠。没有重建 HashMap 桶顺序，也没有把此编码器接成可用 SDK 上下文。

实机纯 Java 验收使用 tools/MetasecWireProbe.java 与独立父加载器版本占位类；先校验固定 APK SHA-256，再确认真正 u2 序列化器来自 APK。u2 是抽象类，因此只通过 X.0mVm 的基类构造初始化假对象，包装引用为 null。版本占位类避免触发原始 s4 的 native 静态初始化；没有初始化应用或加载 SDK 原生库，也没有账号或网络调用。反射仅修改新建的假对象，未访问真实应用进程。首次抽象类实例化及反射访问失败后已修正，最终两个样本均成功。

公开假数据保存为 assets/metasec-fixtures/config-wire.json。Rust 对比默认 17 项数组及配置样本完整 JSON 字节，后者覆盖有符号整型边界、null/空/空白键、数字/布尔/null 值、NBSP、全部 ASCII 控制字符、斜杠、反斜杠及 U+2028/U+2029。手机临时 DEX 均清理，未修改临时目录权限。此验收不涉及原生初始化、请求签名或直播。102 项项目测试通过。

## Rust 摘要与 Helios 部件

通过 GitHub 代码检索定位到 [ShuaiLeiLu/DY_luckybag](https://github.com/ShuaiLeiLu/DY_luckybag/tree/d0fb0bd85b4c83b94b5bb0e16f7a792a06748a12) 的公开研究。其固定 `libmetasec_ml.so` SHA-256 与本项目相同，且恢复了相同模块中的 Medusa、Speck 和 Helios 算术。参考仓库未声明许可证，因此没有复制其源码；Rust 实现只使用算法事实、公开参数与合成测试向量。

`metasec::digest` 实现模块 `0x4420f0` 的 64 轮压缩、非标准填充、16 字节摘要、增量更新和 8/12 字节输入对应的尾部计算，并实现已恢复 Argus 路径的单个小端时间字 Base64 编码。摘要的 F1/F0 后续 SDK 状态位没有伪造，16/20 字节结果不能直接当成最终 X-Medusa 或 X-Argus。

`metasec::speck` 实现 Speck-128/256 的 34 轮密钥扩展、分组加密及原生使用的 PKCS#7 填充；公开算法作者向量与固定轮密钥均通过。`metasec::helios` 按模块 `0x43eb90/F0` 使用 `MD5(nonce || secret)` 的小写十六进制 ASCII 作为 32 字节密钥，加密 `秒时间戳-contextText-aid`，输出 `Base64(nonce || ciphertext)`。调用方必须提供真实 SDK 上下文字符串和秘密材料，当前没有猜测或硬编码这些值。

这些部件已通过合成参考向量，但不是官方 SDK 运行输出对照，更不是服务端签名验收。完整上下文初始化、Medusa 状态位、其余头和最终请求合并仍未完成，生产请求不会调用这些原语。108 项项目测试通过。

## 原生毫秒请求票据

同版本公开官方抓帧及 `SsInterceptor` 静态证据表明，`x-ss-req-ticket` 是请求发送前生成的 `System.currentTimeMillis()` 十进制毫秒值，不是 Metasec 签名，也不能用秒级时间戳替代。Rust 独立请求现已在 `aweme.snssdk.com` 的 Token 账号验证及通用 GET 上添加该头；每次传输重试重新取时。固定 Webcast 权限端点保持原有请求形状，不扩大该规则的适用主机。

本地测试覆盖 1 秒加 234567890 纳秒应生成 `1234`，防止退化为秒值。109 项项目测试通过。该改动只补齐一个已核对的传输头，不证明完整安全头集合、搜索或直播接口已被服务端接受。

## Field 13 三分支摘要

继续通过 GitHub 检索到 [sunnysky123/hongguo-web](https://github.com/sunnysky123/hongguo-web/tree/f458d7455322d4fa98540831477d88429a80b0e5) 的 `pure-sign.js`。该仓库为 MIT，文件明确标注算法源自 GPL-3.0 项目，与当前 AGPL-3.0 项目兼容；没有采用另一份 PolyForm 非商业许可实现的源码。

`metasec::field13` 现支持原始 query 的 SM3、可选正文的 MD5（缺省为 16 字节零）、分支选择，以及全部三种输出：重量级分支的 112 轮消息扩展与查表轮函数、直通分支的 36 字节拼接、变种 MD5 分支的 64 轮压缩和校验和。公开实现生成 9 组固定合成向量，三个分支各三组，Rust 输出逐字节一致；SM3 另通过 `abc` 标准向量。

Field 13 是 Medusa 消息的一个字段，不是完整 X-Medusa。消息其余字段、SDK 会话、外层加密和最终六头合并仍需继续恢复。当前保存的登录态未通过服务端状态检查，本轮没有完成已认证搜索验收。112 项项目测试通过。
